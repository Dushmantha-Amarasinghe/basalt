//! Self-configuration for the host machine.
//!
//! The benchmark has to run on a low-end laptop belonging to someone who is not
//! going to enjoy a twelve-step setup guide. So the host command does as much as
//! it can by itself — opens the firewall, creates the SMB share the baseline
//! needs, works out its own address — and where it genuinely cannot (no admin
//! rights), it prints the exact command to paste rather than a description of
//! what to do.

use std::process::Command;

use anyhow::Result;

/// Name of the SMB share created for the baseline comparison.
pub const SHARE_NAME: &str = "basalt";

/// Firewall rule name, so repeat runs update rather than duplicate it.
const FIREWALL_RULE: &str = "Basalt Benchmark";

/// Runs a PowerShell command, returning stdout on success.
fn powershell(script: &str) -> Option<String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// True if this process can create shares and firewall rules.
pub fn is_elevated() -> bool {
    powershell(
        "([Security.Principal.WindowsPrincipal] \
         [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(\
         [Security.Principal.WindowsBuiltInRole]::Administrator)",
    )
    .map(|s| s.eq_ignore_ascii_case("True"))
    .unwrap_or(false)
}

/// This machine's LAN IPv4 addresses, best first.
///
/// Filters out loopback and link-local. Wi-Fi adapters are preferred because
/// that is how the laptop will actually be reached.
pub fn local_addresses() -> Vec<String> {
    let script = "Get-NetIPAddress -AddressFamily IPv4 | \
         Where-Object { $_.IPAddress -notlike '127.*' -and \
         $_.IPAddress -notlike '169.254.*' } | \
         Sort-Object -Property @{Expression={ if ($_.InterfaceAlias -like '*Wi-Fi*') {0} else {1} }} | \
         ForEach-Object { $_.IPAddress }";

    powershell(script)
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Outcome of a self-configuration step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupStep {
    /// Already configured; nothing to do.
    AlreadyDone,
    /// We changed it successfully.
    Applied,
    /// We could not, and this is the command that would.
    NeedsAdmin(String),
    /// It failed for some other reason.
    Failed(String),
}

impl SetupStep {
    pub fn describe(&self, what: &str) -> String {
        match self {
            SetupStep::AlreadyDone => format!("  {what}: already set up"),
            SetupStep::Applied => format!("  {what}: done"),
            SetupStep::NeedsAdmin(cmd) => {
                format!(
                    "  {what}: NEEDS ADMIN\n      run this in an admin PowerShell:\n      {cmd}"
                )
            }
            SetupStep::Failed(e) => format!("  {what}: failed ({e})"),
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, SetupStep::AlreadyDone | SetupStep::Applied)
    }
}

/// Opens the firewall for the two benchmark ports.
///
/// Without this the client's connection is silently dropped and the failure
/// looks like a network problem rather than a firewall one.
pub fn ensure_firewall(port: u16) -> SetupStep {
    let ports = format!("{port},{}", port + 1);

    let existing = powershell(&format!(
        "if (Get-NetFirewallRule -DisplayName '{FIREWALL_RULE}' \
         -ErrorAction SilentlyContinue) {{ 'yes' }} else {{ 'no' }}"
    ));
    if existing.as_deref() == Some("yes") {
        return SetupStep::AlreadyDone;
    }

    let create = format!(
        "New-NetFirewallRule -DisplayName '{FIREWALL_RULE}' -Direction Inbound \
         -Action Allow -Protocol TCP -LocalPort {ports} -Profile Any"
    );

    if !is_elevated() {
        return SetupStep::NeedsAdmin(create);
    }
    match powershell(&create) {
        Some(_) => SetupStep::Applied,
        None => SetupStep::Failed("could not create the firewall rule".into()),
    }
}

/// Creates a read-only SMB share over the corpus, for the baseline comparison.
pub fn ensure_share(path: &str) -> SetupStep {
    let existing = powershell(&format!(
        "if (Get-SmbShare -Name '{SHARE_NAME}' -ErrorAction SilentlyContinue) \
         {{ 'yes' }} else {{ 'no' }}"
    ));
    if existing.as_deref() == Some("yes") {
        return SetupStep::AlreadyDone;
    }

    // Read-only, and to Everyone: this is a throwaway corpus of generated test
    // data on a home network, and requiring credentials would mean the client
    // machine needs an account on the laptop.
    let create = format!("New-SmbShare -Name '{SHARE_NAME}' -Path '{path}' -ReadAccess 'Everyone'");

    if !is_elevated() {
        return SetupStep::NeedsAdmin(create);
    }
    match powershell(&create) {
        Some(_) => SetupStep::Applied,
        None => SetupStep::Failed("could not create the share".into()),
    }
}

/// Removes what [`ensure_share`] and [`ensure_firewall`] created.
pub fn cleanup() -> Result<()> {
    if !is_elevated() {
        println!(
            "Not running as admin, so nothing was removed. To clean up later, \
             run these in an admin PowerShell:\n  \
             Remove-SmbShare -Name '{SHARE_NAME}' -Force\n  \
             Remove-NetFirewallRule -DisplayName '{FIREWALL_RULE}'"
        );
        return Ok(());
    }
    powershell(&format!(
        "Remove-SmbShare -Name '{SHARE_NAME}' -Force -ErrorAction SilentlyContinue"
    ));
    powershell(&format!(
        "Remove-NetFirewallRule -DisplayName '{FIREWALL_RULE}' -ErrorAction SilentlyContinue"
    ));
    println!("removed the benchmark share and firewall rule");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_step_descriptions_are_actionable() {
        // A "needs admin" result must carry the command to paste, not just say
        // that permission is missing.
        let step = SetupStep::NeedsAdmin("New-SmbShare -Name 'x'".into());
        let text = step.describe("share");
        assert!(
            text.contains("New-SmbShare"),
            "must include the command: {text}"
        );
        assert!(text.contains("admin"), "must say admin is needed: {text}");
        assert!(!step.is_ok());

        assert!(SetupStep::AlreadyDone.is_ok());
        assert!(SetupStep::Applied.is_ok());
        assert!(!SetupStep::Failed("x".into()).is_ok());
    }

    #[test]
    fn elevation_check_returns_without_panicking() {
        // Either answer is valid depending on how the tests were launched; the
        // point is that the query itself is well formed.
        let _ = is_elevated();
    }

    #[test]
    fn local_addresses_look_like_ipv4_and_exclude_loopback() {
        for addr in local_addresses() {
            assert!(
                addr.parse::<std::net::Ipv4Addr>().is_ok(),
                "{addr} is not a valid IPv4 address"
            );
            assert!(!addr.starts_with("127."), "loopback leaked through: {addr}");
            assert!(
                !addr.starts_with("169.254."),
                "link-local leaked through: {addr}"
            );
        }
    }

    #[test]
    fn share_name_is_a_valid_windows_share_name() {
        // Share names cannot contain these, and must be short enough for the
        // legacy limit.
        assert!(!SHARE_NAME.is_empty() && SHARE_NAME.len() <= 80);
        for bad in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!SHARE_NAME.contains(bad), "share name contains {bad}");
        }
    }
}

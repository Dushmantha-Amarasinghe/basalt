//! What the Basalt app needs from Android itself.
//!
//! Most of it is called straight from the interface — the file pickers, the
//! share sheet, keeping the app awake, the screen. What is here in Rust is
//! the part that has to stay out of the page's reach: turning a file the
//! user picked into an open file descriptor, and creating a download in
//! Downloads to write into. The shell calls these; nothing in the page can.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "app.basalt.android";

/// A download being written, and the file to write it into.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewDownload {
    /// Where the system keeps it: a `content://` URI.
    pub uri: String,
    /// Open for writing, detached: whoever takes it owns it.
    pub fd: i32,
    /// Where somebody would find it, for saying so: `Download/Basalt/…`.
    pub shown_as: String,
}

#[derive(Debug, Deserialize)]
struct Fd {
    fd: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenFdArgs<'a> {
    uri: &'a str,
    mode: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateDownloadArgs<'a> {
    name: &'a str,
    mime: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinishDownloadArgs<'a> {
    uri: &'a str,
    ok: bool,
}

#[derive(Debug)]
pub enum Error {
    Unsupported,
    Android(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported => write!(f, "only on Android"),
            Error::Android(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// The Android side, for the shell to call.
pub struct BasaltAndroid<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: tauri::plugin::PluginHandle<R>,
    #[cfg(not(target_os = "android"))]
    _marker: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> BasaltAndroid<R> {
    #[allow(unused_variables)]
    fn call<T: DeserializeOwned>(&self, command: &str, args: impl Serialize) -> Result<T> {
        #[cfg(target_os = "android")]
        {
            self.handle
                .run_mobile_plugin(command, args)
                .map_err(|e| Error::Android(e.to_string()))
        }
        #[cfg(not(target_os = "android"))]
        {
            Err(Error::Unsupported)
        }
    }

    /// Opens a file the user picked or shared, and hands over its descriptor.
    ///
    /// `mode` is `"r"` to read. The descriptor is detached from Android's
    /// wrapper: the caller owns it and must close it — turning it into a
    /// `std::fs::File` does that.
    pub fn open_fd(&self, uri: &str, mode: &str) -> Result<i32> {
        self.call::<Fd>("openFd", OpenFdArgs { uri, mode }).map(|r| r.fd)
    }

    /// Creates `name` in Downloads/Basalt, hidden until finished.
    pub fn create_download(&self, name: &str, mime: &str) -> Result<NewDownload> {
        self.call("createDownload", CreateDownloadArgs { name, mime })
    }

    /// Shows a finished download, or removes one that failed.
    pub fn finish_download(&self, uri: &str, ok: bool) -> Result<()> {
        self.call::<serde_json::Value>("finishDownload", FinishDownloadArgs { uri, ok })
            .map(|_| ())
    }
}

/// Access from an app handle.
pub trait BasaltAndroidExt<R: Runtime> {
    fn basalt_android(&self) -> &BasaltAndroid<R>;
}

impl<R: Runtime, T: Manager<R>> BasaltAndroidExt<R> for T {
    fn basalt_android(&self) -> &BasaltAndroid<R> {
        self.state::<BasaltAndroid<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("basalt-android")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let plugin = BasaltAndroid {
                handle: api.register_android_plugin(PLUGIN_IDENTIFIER, "BasaltPlugin")?,
            };
            #[cfg(not(target_os = "android"))]
            let plugin = {
                let _ = api;
                BasaltAndroid::<R> {
                    _marker: std::marker::PhantomData,
                }
            };
            app.manage(plugin);
            Ok(())
        })
        .build()
}

# How to run the speed test

This measures whether custom software would actually move files faster than
Windows' built-in file sharing. Two commands, one on each machine.

You need: the old laptop, the hard drive, both on the same Wi-Fi.

---

## Step 1 — get the program onto the laptop

Copy this one file across (USB stick is easiest):

```
dist\basalt-bench.exe
```

Nothing to install. It runs on its own — no Rust, no runtime, nothing.

## Step 2 — plug the hard drive into the laptop

Note the drive letter Windows gives it. Open **This PC** and look. It'll be
something like `D:` or `E:`.

## Step 3 — on the LAPTOP, open PowerShell as administrator

Press Start, type `powershell`, then **right-click** "Windows PowerShell" and
choose **Run as administrator**.

This matters: without administrator the program can't create the Windows share,
and the Windows share is the thing we're comparing against.

Then run this, replacing `E:` with your actual drive letter and the path to
where you put the exe:

```powershell
C:\Users\you\Desktop\basalt-bench.exe host --root E:\bench-corpus
```

It will:
- create test files on the drive (**this takes 5–15 minutes**, once only)
- open the firewall
- create the Windows share
- print a command for you to run on your PC

**Leave this window open.** It has to keep running.

## Step 4 — on YOUR PC, run the command it printed

It'll look like this:

```powershell
basalt-bench.exe measure --host 192.168.1.50
```

Takes a few minutes. When it finishes it prints the answer in plain English —
which is faster, by how much, and whether the custom software is worth building.

## Step 5 — when you're done

On the laptop, press **Ctrl-C** to stop it, then:

```powershell
basalt-bench.exe cleanup
```

That removes the Windows share and the firewall rule. You can also delete the
`bench-corpus` folder from the drive — it's only test data.

---

## If something goes wrong

**"Could not reach the benchmark server"**
The laptop's firewall is blocking it. Check the laptop window said
`Firewall: done` and not `NEEDS ADMIN`. If it said NEEDS ADMIN, you didn't open
PowerShell as administrator — go back to step 3.

**"Could not measure Windows sharing"**
Open `\\LAPTOP-NAME\basalt` in File Explorer on your PC once, so Windows
authenticates. Then run step 4 again.

**It says the address doesn't work**
The laptop prints more than one address if it has several network adapters. Try
the others it listed. The right one usually starts `192.168.`.

**The laptop is really slow making the test files**
Expected on a spinning USB drive. Add `--quick` to step 3 for a smaller set —
the numbers are slightly less reliable but still useful.

---

## What you'll get

A result like this:

```
  Thousands of small files
    Basalt             48.2 MB/s
    Windows sharing    12.1 MB/s
    -> Basalt is 3.98x faster

  BUILD IT — the custom protocol is clearly faster
```

or the opposite:

```
  USE WINDOWS SHARING — the custom protocol is not faster
```

Either answer is useful. The second one saves months of work.

Full numbers land in `docs\benchmarks.md` on your PC.

## Which download?

| Your computer | Download |
|---|---|
| Windows (Intel/AMD) | `…-windows-x86_64.zip` |
| Windows on ARM | `…-windows-arm64.zip` |
| Mac with Apple silicon (M1 and later) | `…-macos-arm64.tar.gz` |
| Mac with an Intel chip | `…-macos-x86_64.tar.gz` |
| Linux (Intel/AMD) | `…-linux-x86_64.tar.gz` |
| Linux on ARM | `…-linux-arm64.tar.gz` |

Unpack it and run `atom-rs` (`atom-rs.exe` on Windows). The ROMs are in the `roms` folder beside it. `SHA256SUMS.txt` has the checksums.

## Requirements

* **Linux:** **glibc 2.35 or newer**, which means Ubuntu 22.04 or later, Debian 12 or later, Fedora 36 or later, or similar (check with `ldd --version`). Also the ALSA sound library (`libasound2`, installed on most desktops) and an X11 or Wayland desktop. Built on Ubuntu 22.04.
* **macOS:** macOS 11 or later on Apple silicon. The Intel build is marked for macOS 10.12 or later, but has only been tried on recent versions. The program is not signed, so run `xattr -cr` on the unpacked folder once, or right-click and choose Open.
* **Windows:** Windows 10 or 11. SmartScreen may warn about an unrecognised app: choose *More info*, then *Run anyway*.

Run `atom-rs --archive` from a terminal to download the Atom Software Archive (about 1,100 programs) and open its menu. The [README](https://github.com/harrowm/atom-rs#readme) has the rest.

## What changed

//! Command-line options.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = r#"Acorn Atom emulator

Usage: atom-rs [OPTIONS] [ROM_FOLDER]

Options:
  --disk FILE      put a disk image (.ssd or .dsd) in drive 0
  --disk1 FILE     put a disk image in drive 1
  --sdcard FOLDER  use a folder as an AtoMMC SD card (needs roms/ramrom.rom).
                   Try the archive's menu with --run '*MENU\n'
  --archive        fetch the latest Atom Software Archive (the first time, and
                   whenever a newer one is published), use it as the SD card
                   and start its menu. Needs roms/ramrom.rom
  --tape FILE      insert a tape (.uef). It plays by itself when the Atom
                   says PLAY TAPE
  --state FILE     start from a saved state (.atomstate)
  --run TEXT       type TEXT once the Atom has started ("\n" is RETURN).
                   With --disk, "*DOS" is typed first if you did not
  --fast-tape      run at maximum speed while the tape is playing
  --debug          open the debugger windows
  --help           show this help

ROM_FOLDER defaults to ./roms.

Examples:
  atom-rs --archive
  atom-rs --sdcard testdata/asa_v13_01/files --run '*MENU\n'
  atom-rs --disk games.ssd --run '*RUN CHUCKIE\n'
  atom-rs --tape chuckie.uef --fast-tape --run '*RUN ""\n'
"#;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub rom_dir: Option<PathBuf>,
    pub debug: bool,
    pub disks: [Option<PathBuf>; 2],
    /// A folder to use as the AtoMMC SD card.
    pub sdcard: Option<PathBuf>,
    /// Fetch the Atom Software Archive and use it as the SD card.
    pub archive: bool,
    pub tape: Option<PathBuf>,
    pub state: Option<PathBuf>,
    /// Text to type after start-up, with `\n` already turned into newlines.
    pub run: Option<String>,
    pub fast_tape: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Box<Options>),
    Help,
}

/// Read the options from `args` (not including the program name).
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Parsed, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        let flag = arg.to_string_lossy().into_owned();
        // Flags that take a value.
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} needs a value (see --help)"))
        };
        match flag.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--debug" => options.debug = true,
            "--fast-tape" => options.fast_tape = true,
            "--disk" => options.disks[0] = Some(value("--disk")?.into()),
            "--disk1" => options.disks[1] = Some(value("--disk1")?.into()),
            "--archive" => options.archive = true,
            "--sdcard" => options.sdcard = Some(value("--sdcard")?.into()),
            "--tape" => options.tape = Some(value("--tape")?.into()),
            "--state" => options.state = Some(value("--state")?.into()),
            "--run" => {
                options.run = Some(unescape(&value("--run")?.to_string_lossy()));
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other} (see --help)"));
            }
            _ => options.rom_dir = Some(PathBuf::from(arg)),
        }
    }
    Ok(Parsed::Run(Box::new(options)))
}

/// Turn the two characters `\n` into a newline and `\\` into a backslash, so
/// commands can be written on one line in a shell.
fn unescape(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('\\') => result.push('\\'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Parsed, String> {
        parse(args.iter().map(OsString::from))
    }

    fn options(args: &[&str]) -> Options {
        match parse_strs(args).unwrap() {
            Parsed::Run(options) => *options,
            Parsed::Help => panic!("unexpected help"),
        }
    }

    #[test]
    fn no_arguments_gives_defaults() {
        assert_eq!(options(&[]), Options::default());
    }

    #[test]
    fn reads_every_option() {
        let o = options(&[
            "--debug",
            "--fast-tape",
            "--archive",
            "--disk",
            "a.ssd",
            "--disk1",
            "b.dsd",
            "--sdcard",
            "card",
            "--tape",
            "t.uef",
            "--state",
            "s.atomstate",
            "--run",
            "*CAT\\n",
            "my/roms",
        ]);
        assert!(o.debug && o.fast_tape && o.archive);
        assert_eq!(o.disks, [Some("a.ssd".into()), Some("b.dsd".into())]);
        assert_eq!(o.sdcard, Some("card".into()));
        assert_eq!(o.tape, Some("t.uef".into()));
        assert_eq!(o.state, Some("s.atomstate".into()));
        assert_eq!(o.run.as_deref(), Some("*CAT\n"));
        assert_eq!(o.rom_dir, Some("my/roms".into()));
    }

    #[test]
    fn help_and_errors() {
        assert_eq!(parse_strs(&["--help"]), Ok(Parsed::Help));
        assert!(
            parse_strs(&["--disk"])
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            parse_strs(&["--bogus"])
                .unwrap_err()
                .contains("unknown option")
        );
    }

    #[test]
    fn unescape_handles_newlines_and_backslashes() {
        assert_eq!(unescape(r#"*RUN ""\n"#), "*RUN \"\"\n");
        assert_eq!(unescape(r"A\nB"), "A\nB");
        assert_eq!(unescape(r"A\\nB"), "A\\nB");
        assert_eq!(unescape(r"trailing\"), "trailing\\");
    }
}

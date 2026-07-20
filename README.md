```
       __            __________
  ____/ /_  ______  / / __/ __/
 / __  / / / / __ \/ / /_/ /_
/ /_/ / /_/ / /_/ / / __/ __/
\__,_/\__,_/ .___/_/_/ /_/
          /_/
```

Fast duplicate file finder with a terminal UI, a desktop GUI, and scriptable
output modes.

## How it works

duplff walks the directories you give it, groups files by size, then confirms
duplicates with BLAKE3 hashing. A cheap 4KB partial hash filters candidates
before the full-file hash runs. Each duplicate group gets a keep
recommendation based on your priority folders, path depth, and modification
time. Deletions always go to the OS trash, never straight to disk, and every
deletion batch is logged so it can be undone.

## Install

See [INSTALL.md](INSTALL.md) for the `.deb` package, AppImage, and CLI-only
installs.

## Usage

```
Usage: duplff [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  Directories to scan for duplicates

Options:
  -e, --ext <EXTENSIONS>     File extensions to include (e.g. py rs js)
  -m, --min-size <MIN_SIZE>  Minimum file size in bytes (default: 1)
  -M, --max-size <MAX_SIZE>  Maximum file size in bytes
  -p, --priority <PRIORITY>  Priority directories (files here are preferred to keep)
  -x, --exclude <EXCLUDE>    Exclude directories/patterns (glob, repeatable)
  -L, --follow-symlinks      Follow symbolic links
      --json                 Output JSON report (non-interactive)
      --dry-run              Show deletion plan without deleting (non-interactive)
      --csv                  Output CSV report (non-interactive)
      --no-cache             Disable hash cache
      --paranoid             Byte-by-byte verification after hash match
  -h, --help                 Print help
  -V, --version              Print version
```

Running `duplff <paths>` with no output flag launches the interactive TUI.
Press `?` inside for key bindings. The `--json`, `--csv`, and `--dry-run`
flags print a report to stdout instead, which makes duplff easy to script.

## Features

- **Safe by default.** Files are moved to the OS trash and each batch is
  logged for undo. Paranoid mode adds a byte-by-byte comparison on top of
  hash matching.
- **Fast rescans.** A SQLite hash cache keyed by path, size, and mtime skips
  rehashing unchanged files. Disable it with `--no-cache`.
- **Keep-file ranking.** Priority directories win, then deeper paths, then
  newer files, with a lexicographic tiebreaker so results are deterministic.
- **Desktop GUI.** A Tauri app with folder pickers, progress, filtering,
  undo, and JSON/CSV export, built on the same core library.

## Documentation

- [duplff-core](docs/duplff-core.md): the scanning and hashing pipeline
- [duplff-cli](docs/duplff-cli.md): TUI and non-interactive modes
- [duplff-gui](docs/duplff-gui.md): the desktop app

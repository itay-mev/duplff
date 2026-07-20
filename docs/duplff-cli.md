# duplff-cli

Terminal interface for duplff. Supports both an interactive TUI and non-interactive output modes.

## Interactive TUI

Run with just paths to launch the TUI:

```
duplff ~/Documents ~/Downloads
```

The TUI has two screens:

- **Scan** shows live progress: files found while walking, then hashing progress.
- **Results** shows a table of duplicate groups next to a detail pane for the
  selected group. Groups are sorted by wasted space by default. Mark duplicates
  in the detail pane, then press `d` to move everything marked to the trash
  after a confirmation prompt.

### Key Bindings

| Key | Action |
|-----|--------|
| `q` | Quit |
| `Esc` | Clear the active filter, or quit if none |
| `j` / `k` / `↑` / `↓` | Navigate |
| `Tab` | Switch between groups and detail panes |
| `Enter` | Open the selected group's detail pane |
| `Space` | Toggle deletion mark on the selected duplicate |
| `D` | Mark every duplicate in the current group |
| `u` | Unmark every duplicate in the current group |
| `d` | Trash marked files (asks for confirmation) |
| `s` | Cycle sort mode (wasted, size, files, path) |
| `/` | Filter groups by path substring (Enter applies, Esc clears) |
| `?` | Help overlay |

The keep file in each group can never be marked for deletion.

## Non-Interactive Modes

```
duplff --json ~/Documents         # JSON report to stdout
duplff --csv ~/Documents          # CSV report to stdout
duplff --dry-run ~/Documents      # Show what would be deleted
```

## Common Options

```
-e py rs js          # Only scan these extensions
-m 1024              # Min file size (bytes)
-M 10485760          # Max file size (bytes)
-p ~/important       # Priority directory (prefer keeping files here)
-x node_modules      # Exclude pattern (repeatable)
-L                   # Follow symlinks
--paranoid           # Byte-by-byte verification
--no-cache           # Disable hash cache
```

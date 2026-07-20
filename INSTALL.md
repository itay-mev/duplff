# Installing duplff

## Debian / Ubuntu

### Install

Download the `.deb` package from the [latest release](https://github.com/itay-mev/duplff/releases/latest), then install it:

```bash
sudo dpkg -i duplff_*_amd64.deb
```

If there are missing dependencies:

```bash
sudo apt-get install -f
```

This installs both the **GUI app** (launch from your application menu or run `duplff-gui`) and the **CLI tool** (`duplff`).

### Uninstall

```bash
sudo dpkg -r duplff
```

To also remove any configuration files:

```bash
sudo dpkg --purge duplff
```

### Alternative: AppImage (no install needed)

Download the `.AppImage` file from the [latest release](https://github.com/itay-mev/duplff/releases/latest):

```bash
chmod +x duplff_*_amd64.AppImage
./duplff_*_amd64.AppImage
```

No installation or root access required. Delete the file to remove it.

### CLI-only

If you only want the command-line tool without the GUI:

```bash
# Download the binary
curl -L -o duplff https://github.com/itay-mev/duplff/releases/latest/download/duplff
chmod +x duplff

# Move to a directory in your PATH
sudo mv duplff /usr/local/bin/

# Verify
duplff --version
```

To uninstall:

```bash
sudo rm /usr/local/bin/duplff
```

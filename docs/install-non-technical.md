# Fullbleed Install Guide (Non-Technical)

This guide is for people who are new to Python.

Goal: install Python, install Fullbleed, and generate your first PDF.

## 1. Install Python

Use Python `3.11` (64-bit) if you are unsure. Fullbleed requires Python `3.8+`.

### Windows

1. Open `https://www.python.org/downloads/windows/`.
2. Download Python `3.11.x` (64-bit).
3. Run the installer.
4. Check `Add python.exe to PATH`.
5. Click `Install Now`.
6. Close PowerShell and open it again.

### macOS

1. Open `https://www.python.org/downloads/macos/`.
2. Download Python `3.11.x`.
3. Run the installer package.
4. Close Terminal and open it again.

### Linux (Ubuntu/Debian)

Open Terminal and run:

```bash
sudo apt update
sudo apt install -y python3 python3-pip
```

## 2. Verify Python and pip

### Windows

```bash
python --version
python -m pip --version
```

### macOS/Linux

```bash
python3 --version
python3 -m pip --version
```

If `python` is not found, use `python3` in all commands below.

## 3. Install Fullbleed

### Windows

```bash
python -m pip install --upgrade pip
python -m pip install fullbleed
```

### macOS/Linux

```bash
python3 -m pip install --upgrade pip
python3 -m pip install fullbleed
```

## 4. Verify Fullbleed

Try:

```bash
fullbleed --help
```

If `fullbleed` is not recognized, use:

```bash
python -m fullbleed --help
```

or:

```bash
python3 -m fullbleed --help
```

## 5. Create your first PDF

Create a folder and run Fullbleed scaffold:

### Windows

```bash
mkdir my-first-fullbleed
cd my-first-fullbleed
fullbleed init .
python report.py
```

### macOS/Linux

```bash
mkdir my-first-fullbleed
cd my-first-fullbleed
fullbleed init .
python3 report.py
```

Expected output file:

- `output/report.pdf`

## 6. Common fixes

- Error: `python is not recognized`
  - Re-run the Python installer and check `Add python.exe to PATH`, then restart terminal.
- Error: `No module named pip`
  - Run `python -m ensurepip --upgrade` (or `python3 -m ensurepip --upgrade`).
- Error mentions `Rust`, `cargo`, or building a wheel
  - Upgrade pip first, then retry install.
  - Use Python `3.11` 64-bit to avoid unsupported interpreter builds.
- Error: `fullbleed is not recognized`
  - Restart terminal.
  - Use `python -m fullbleed --help` (or `python3 -m fullbleed --help`).
- Permission error when running pip
  - Use `python -m pip install --user fullbleed` (or `python3 -m pip install --user fullbleed`).

## 7. Install from a local wheel (optional)

If someone gives you a `.whl` file directly:

```bash
python -m pip install C:\path\to\fullbleed-0.5.0-cp311-cp311-win_amd64.whl
```

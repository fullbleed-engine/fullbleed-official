use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=FULLBLEED_PYTHON_LIB_DIR");
    println!("cargo:rerun-if-env-changed=FULLBLEED_PYTHON_SYS_EXECUTABLE");
    println!("cargo:rerun-if-env-changed=PYTHON_SYS_EXECUTABLE");

    if env::var_os("CARGO_FEATURE_PYTHON").is_none() {
        return;
    }

    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => configure_windows_python(),
        Ok("macos") => configure_macos_extension(),
        _ => {}
    }
}

fn configure_windows_python() {
    let library_dir = env::var_os("FULLBLEED_PYTHON_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(discover_windows_python_library_dir);
    let stable_import_library = library_dir.join("python3.lib");
    if !stable_import_library.is_file() {
        panic!(
            "Fullbleed's stable Python ABI requires {}; set \
             FULLBLEED_PYTHON_LIB_DIR to the target Python installation's libs directory",
            stable_import_library.display()
        );
    }
    println!("cargo:rustc-link-search=native={}", library_dir.display());
}

fn discover_windows_python_library_dir() -> PathBuf {
    let interpreter = env::var_os("FULLBLEED_PYTHON_SYS_EXECUTABLE")
        .or_else(|| env::var_os("PYTHON_SYS_EXECUTABLE"))
        .unwrap_or_else(|| "python".into());
    let script = concat!(
        "import pathlib, sys, sysconfig\n",
        "candidates = [\n",
        "    sysconfig.get_config_var('LIBDIR'),\n",
        "    pathlib.Path(sys.base_prefix) / 'libs',\n",
        "    pathlib.Path(sys.prefix) / 'libs',\n",
        "]\n",
        "for candidate in candidates:\n",
        "    if candidate and (pathlib.Path(candidate) / 'python3.lib').is_file():\n",
        "        print(pathlib.Path(candidate).resolve())\n",
        "        break\n",
    );
    let output = Command::new(&interpreter)
        .args(["-c", script])
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to run Python interpreter {:?} while locating python3.lib: {error}",
                interpreter
            )
        });
    if !output.status.success() {
        panic!(
            "Python interpreter {:?} failed while locating python3.lib: {}",
            interpreter,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let path = String::from_utf8(output.stdout)
        .expect("Python library directory was not valid UTF-8")
        .trim()
        .to_owned();
    if path.is_empty() {
        panic!(
            "could not locate python3.lib through {:?}; set FULLBLEED_PYTHON_LIB_DIR",
            interpreter
        );
    }
    let path = Path::new(&path);
    path.to_path_buf()
}

fn configure_macos_extension() {
    // Extension modules resolve CPython symbols from their loading interpreter.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-undefined");
    println!("cargo:rustc-cdylib-link-arg=-Wl,dynamic_lookup");
}

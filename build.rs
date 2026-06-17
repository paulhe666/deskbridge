use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/deskbridge.ico");
    println!("cargo:rerun-if-changed=packaging/windows/deskbridge.rc");

    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }

    let Some(rc) = find_tool(&["rc.exe", "rc"]) else {
        println!(
            "cargo:warning=rc.exe not found; Windows exe icon will be added by the installer when possible"
        );
        return;
    };

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let resource = out_dir.join("deskbridge.res");
    let status = Command::new(rc)
        .arg("/nologo")
        .arg(format!("/fo{}", resource.display()))
        .arg("packaging/windows/deskbridge.rc")
        .status();

    match status {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg-bin=deskbridge={}", resource.display());
        }
        Ok(status) => {
            println!(
                "cargo:warning=rc.exe failed with status {status}; Windows exe icon was not embedded"
            );
        }
        Err(e) => {
            println!("cargo:warning=failed to run rc.exe: {e}; Windows exe icon was not embedded");
        }
    }
}

fn find_tool(candidates: &'static [&'static str]) -> Option<&'static str> {
    candidates.iter().copied().find(|tool| {
        Command::new(tool)
            .arg("/?")
            .status()
            .map(|status| status.success() || status.code().is_some())
            .unwrap_or(false)
    })
}

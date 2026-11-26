use std::borrow::Cow;
use std::env;
use std::error::Error;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use vergen::Emitter;
use vergen::{Build, Cargo};
use vergen_gitcl::Gitcl;

fn main() -> Result<(), Box<dyn Error>> {
    // Emit version info, and "rerun-if-changed" for relevant files, including build.rs
    let build = Build::builder().build_timestamp(true).build();
    let cargo = Cargo::builder().features(true).build();
    let gitcl = Gitcl::builder()
        .sha(false)
        .commit_timestamp(true)
        .commit_date(true)
        .build();
    Emitter::default()
        .default_on_error()
        .add_instructions(&build)?
        .add_instructions(&cargo)?
        .add_instructions(&gitcl)?
        .emit()?;

    // Embed resource file w/ icon and version info on Windows.
    // Note: We check CARGO_CFG_TARGET_OS rather than cfg! because build.rs
    // runs on the host, not the target.
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS not set");
    if target_os == "windows" {
        set_windows_resource()?;
        embed_battle_icons()?;
    }

    println!("cargo:rerun-if-env-changed=CFG_RELEASE_CHANNEL");
    println!(
        "cargo:rustc-env=CFG_RELEASE_CHANNEL={channel}",
        channel = get_channel()
    );

    // Some SWFS have a large amount of recursion (particularly
    // around `goto`s). Increase the stack size on Windows
    // accommodate this (the default on Linux is high enough). We
    // do the same thing for wasm in web/build.rs.
    let target = std::env::var("TARGET").expect("TARGET not set");

    if target.contains("windows") {
        if target.contains("msvc") {
            println!("cargo:rustc-link-arg=/STACK:4000000");
        } else {
            println!("cargo:rustc-link-arg=-Xlinker");
            println!("cargo:rustc-link-arg=--stack");
            println!("cargo:rustc-link-arg=4000000");
        }
    }

    Ok(())
}

fn get_channel() -> Cow<'static, str> {
    if let Ok(channel) = env::var("CFG_RELEASE_CHANNEL") {
        Cow::Owned(channel)
    } else {
        Cow::Borrowed("local")
    }
}

fn embed_battle_icons() -> Result<(), Box<dyn Error>> {
    let assets = Path::new("assets/battle-assistant");
    println!("cargo:rerun-if-changed={}", assets.display());
    let mut icons = Vec::new();
    for category in ["pet", "equipment", "soul-card", "soul-artifact", "crystal-key"] {
        for entry in fs::read_dir(assets.join(category))? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("png") {
                let id = path.file_stem().and_then(|s| s.to_str()).ok_or("Invalid icon name")?;
                if !id.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(format!("Invalid battle icon ID: {id}").into());
                }
                icons.push(format!("{category}/{id}.png"));
            }
        }
    }
    icons.sort_unstable();
    let mut table = String::from("&[\n");
    for icon in icons {
        writeln!(table, r#"("/battle-assets/{icon}", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/battle-assistant/{icon}"))),"#)?;
    }
    table.push_str("]\n");
    fs::write(Path::new(&env::var("OUT_DIR")?).join("battle_icons.rs"), table)?;
    Ok(())
}

fn set_windows_resource() -> Result<(), Box<dyn Error>> {
    let mut res = winresource::WindowsResource::new();

    // Set language to US English (0x0409)
    res.set_language(0x0409);

    // Set the application icon
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let icon_path = format!("{manifest_dir}/assets/favicon.ico");

    println!("cargo:rerun-if-changed={icon_path}");

    res.set_icon(&icon_path);

    // Set debug flag when building in debug mode
    if let Ok(profile) = env::var("PROFILE")
        && profile == "debug"
    {
        res.set_version_info(
            winresource::VersionInfo::FILEFLAGS,
            winresource::VersionInfo::VS_FF_DEBUG,
        );
    }

    res.compile()?;

    Ok(())
}

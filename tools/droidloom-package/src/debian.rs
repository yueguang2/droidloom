//! Convert the matching pacman package pair into Debian packages.
//!
//! The published Arch packages are the single build product. Debian and Ubuntu
//! use the same portable host binaries and Android image; this module only
//! changes the package container and lifecycle metadata.
use super::{Result, repository, run};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const RUNTIME: &str = "droidloom-runtime";
const IMAGE: &str = "droidloom-image";
const MAINTAINER: &str = "Doctor Logix <doctor.logix@gmail.com>";
const HOMEPAGE: &str = "https://github.com/denialwm/droidloom";

#[derive(Debug, Default)]
struct PacmanInfo {
    name: String,
    version: String,
    architecture: String,
    description: String,
    installed_size: u64,
    build_date: u64,
    depends: Vec<String>,
    optional_depends: Vec<String>,
}

impl PacmanInfo {
    fn parse(text: &str) -> Result<Self> {
        let mut info = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(" = ") else {
                continue;
            };
            match key {
                "pkgname" => info.name = value.into(),
                "pkgver" => info.version = value.into(),
                "arch" => info.architecture = value.into(),
                "pkgdesc" => info.description = value.into(),
                "size" => info.installed_size = value.parse()?,
                "builddate" => info.build_date = value.parse()?,
                "depend" => info.depends.push(value.into()),
                "optdepend" => info.optional_depends.push(value.into()),
                _ => {}
            }
        }
        if info.name.is_empty()
            || info.version.is_empty()
            || info.architecture.is_empty()
            || info.description.is_empty()
        {
            return Err("package metadata is incomplete".into());
        }
        Ok(info)
    }
}

fn pacman_info(archive: &Path) -> Result<PacmanInfo> {
    let output = Command::new("tar")
        .args(["--zstd", "-xOf"])
        .arg(archive)
        .arg(".PKGINFO")
        .output()?;
    if !output.status.success() {
        return Err(format!("{} has no .PKGINFO", archive.display()).into());
    }
    PacmanInfo::parse(&String::from_utf8(output.stdout)?)
}

fn archives(directory: &Path) -> Result<(PathBuf, PathBuf)> {
    let mut runtime = Vec::new();
    let mut image = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".pkg.tar.zst") {
            continue;
        }
        if name.starts_with("droidloom-runtime-") {
            runtime.push(path);
        } else if name.starts_with("droidloom-image-") {
            image.push(path);
        }
    }
    if runtime.len() != 1 || image.len() != 1 {
        return Err(format!(
            "expected exactly one runtime and one image pacman package in {}",
            directory.display()
        )
        .into());
    }
    Ok((runtime.remove(0), image.remove(0)))
}

fn debian_architecture(architecture: &str) -> Result<&'static str> {
    match architecture {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        other => Err(format!("no Debian architecture for {other}").into()),
    }
}

/// Translate one pacman dependency into its Debian/Ubuntu equivalent.
fn dependency(name_and_version: &str) -> Result<String> {
    let (name, version) = name_and_version
        .split_once('=')
        .map_or((name_and_version, None), |(name, version)| {
            (name, Some(version))
        });
    let mapped = match name {
        "droidloom-image" => {
            let version = version.ok_or("droidloom-image dependency has no version")?;
            format!("droidloom-image (= {version})")
        }
        "glibc" => "libc6 (>= 2.39)".into(),
        "gcc-libs" => "libgcc-s1".into(),
        "mesa" => "libgbm1, libgl1-mesa-dri".into(),
        "libdrm" => "libdrm2".into(),
        // Arch's `wayland` package provides both libraries; aarch64 host
        // binaries link the server side, x86_64 binaries link the client side.
        "wayland" => "libwayland-client0, libwayland-server0".into(),
        "libxkbcommon" => "libxkbcommon0".into(),
        "libpipewire" => "libpipewire-0.3-0".into(),
        "pipewire" => "pipewire".into(),
        "systemd" => "systemd".into(),
        "polkit" => "polkitd, pkexec".into(),
        "util-linux" => "util-linux".into(),
        "e2fsprogs" => "e2fsprogs".into(),
        "iproute2" => "iproute2".into(),
        "nftables" => "nftables".into(),
        "iptables" => "iptables".into(),
        "android-tools" => "adb | android-tools-adb".into(),
        "vulkan-icd-loader" => "libvulkan1".into(),
        "tar" => "tar".into(),
        "zstd" => "zstd".into(),
        other => return Err(format!("no Debian package mapping for {other}").into()),
    };
    Ok(mapped)
}

fn recommends(info: &PacmanInfo) -> Result<Vec<String>> {
    let mut result = Vec::new();
    for dependency in &info.optional_depends {
        let name = dependency.split(':').next().unwrap_or(dependency);
        if name == "wireplumber" {
            result.push("wireplumber".into());
        }
    }
    result.push("mesa-vulkan-drivers".into());
    Ok(result)
}

fn control(
    package: &str,
    info: &PacmanInfo,
    architecture: &str,
    dependencies: &[String],
    recommends: &[String],
    description: &str,
) -> String {
    let installed_size = info.installed_size.div_ceil(1024);
    let mut text = format!(
        "Package: {package}\nVersion: {}\nArchitecture: {architecture}\nMaintainer: {MAINTAINER}\nInstalled-Size: {installed_size}\n",
        info.version
    );
    if !dependencies.is_empty() {
        text.push_str("Depends: ");
        text.push_str(&dependencies.join(", "));
        text.push('\n');
    }
    if !recommends.is_empty() {
        text.push_str("Recommends: ");
        text.push_str(&recommends.join(", "));
        text.push('\n');
    }
    text.push_str(&format!(
        "Section: graphics\nPriority: optional\nHomepage: {HOMEPAGE}\nDescription: {}\n {description}\n",
        info.description
    ));
    text
}

fn extract(archive: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
    run(Command::new("tar")
        .args(["--zstd", "-xpf"])
        .arg(archive)
        .args([
            "--no-same-owner",
            "--exclude=.PKGINFO",
            "--exclude=.BUILDINFO",
            "--exclude=.MTREE",
            "--exclude=.INSTALL",
            "--exclude=usr/share/libalpm",
            "-C",
        ])
        .arg(destination))?;
    Ok(())
}

fn add_documentation(root: &Path, package: &str, info: &PacmanInfo) -> Result<()> {
    let documentation = root.join(format!("usr/share/doc/{package}"));
    fs::create_dir_all(&documentation)?;
    fs::set_permissions(&documentation, fs::Permissions::from_mode(0o755))?;
    if let Some(parent) = documentation.parent() {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755))?;
    }
    let license = root.join(format!("usr/share/licenses/{package}/LICENSE"));
    let mut copyright =
        format!("Droidloom is distributed under GPL-3.0-or-later.\n\nSource: {HOMEPAGE}\n\n");
    if license.is_file() {
        copyright.push_str(&fs::read_to_string(license)?);
    } else {
        copyright.push_str("See /usr/share/licenses for the complete license text.\n");
    }
    fs::write(documentation.join("copyright"), copyright)?;
    fs::set_permissions(
        documentation.join("copyright"),
        fs::Permissions::from_mode(0o644),
    )?;
    let changelog = format!(
        "{package} ({}) unstable; urgency=medium\n\n  * Droidloom {} preview release.\n\n -- {MAINTAINER}\n",
        info.version, info.version
    );
    fs::write(documentation.join("changelog"), changelog)?;
    fs::set_permissions(
        documentation.join("changelog"),
        fs::Permissions::from_mode(0o644),
    )?;
    Ok(())
}

fn write_scripts(source: &Path, destination: &Path, package: &str) -> Result<()> {
    fs::create_dir_all(destination)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
    for script in ["preinst", "postinst", "prerm", "postrm"] {
        let path = source.join(format!("{package}.{script}"));
        if !path.is_file() {
            return Err(format!("missing maintainer script {}", path.display()).into());
        }
        let target = destination.join(script);
        fs::copy(&path, &target)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn tar_control(control: &Path, output: &Path, build_date: u64) -> Result<()> {
    run(Command::new("tar")
        .args([
            "-czf",
            output.to_str().ok_or("non-UTF8 control archive path")?,
            "--sort=name",
            &format!("--mtime=@{build_date}"),
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "-C",
        ])
        .arg(control)
        .arg("."))?;
    Ok(())
}

fn tar_data(root: &Path, output: &Path, build_date: u64) -> Result<()> {
    run(Command::new("tar")
        .args([
            "--zstd",
            "-cf",
            output.to_str().ok_or("non-UTF8 data archive path")?,
            "--sort=name",
            &format!("--mtime=@{build_date}"),
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "-C",
        ])
        .arg(root)
        .arg("."))?;
    Ok(())
}

fn stage(
    archive: &Path,
    package: &str,
    info: &PacmanInfo,
    repository_root: &Path,
    work: &Path,
    description: &str,
) -> Result<()> {
    let root = work.join(format!("{package}-root"));
    extract(archive, &root)?;
    add_documentation(&root, package, info)?;
    let control_dir = work.join(format!("{package}-control"));
    write_scripts(
        &repository_root.join("packaging/debian/scripts"),
        &control_dir,
        package,
    )?;
    let mut dependencies: Vec<_> = info
        .depends
        .iter()
        .map(|d| dependency(d))
        .collect::<Result<_>>()?;
    if package == RUNTIME {
        // Linked directly by the host binaries but not named in pacman's
        // dependency list because Arch's mesa package pulls it in.
        dependencies.push("libexpat1".into());
    }
    let control_text = control(
        package,
        info,
        debian_architecture(&info.architecture)?,
        &dependencies,
        &recommends(info)?,
        description,
    );
    fs::write(control_dir.join("control"), control_text)?;
    fs::set_permissions(
        control_dir.join("control"),
        fs::Permissions::from_mode(0o644),
    )?;
    Ok(())
}

fn assemble(work: &Path, package: &str, output: &Path, build_date: u64) -> Result<()> {
    let control = work.join(format!("{package}-control"));
    let root = work.join(format!("{package}-root"));
    // Debian member names are fixed and short; GNU ar would otherwise add a
    // long-name table before debian-binary, which dpkg rejects.
    let archive = work.join(format!("{package}-archive"));
    fs::create_dir_all(&archive)?;
    let control_tar = archive.join("control.tar.gz");
    let data_tar = archive.join("data.tar.zst");
    tar_control(&control, &control_tar, build_date)?;
    tar_data(&root, &data_tar, build_date)?;
    fs::write(archive.join("debian-binary"), "2.0\n")?;
    if output.exists() {
        fs::remove_file(output)?;
    }
    run(Command::new("ar")
        .current_dir(&archive)
        .arg("rc")
        .arg(output)
        .args(["debian-binary", "control.tar.gz", "data.tar.zst"]))?;
    Ok(())
}

/// Build `droidloom-runtime` and `droidloom-image` `.deb` files from one
/// matching pacman package pair.
pub fn convert(packages: &Path, destination: Option<&Path>) -> Result<()> {
    let repository = repository(None)?;
    let packages = packages.canonicalize()?;
    let (runtime_archive, image_archive) = archives(&packages)?;
    let runtime = pacman_info(&runtime_archive)?;
    let image = pacman_info(&image_archive)?;
    if runtime.name != RUNTIME || image.name != IMAGE {
        return Err(format!(
            "expected {RUNTIME} and {IMAGE} packages, found {} and {}",
            runtime.name, image.name
        )
        .into());
    }
    if runtime.version != image.version {
        return Err(format!(
            "package versions differ: {} and {}",
            runtime.version, image.version
        )
        .into());
    }
    if runtime.architecture != image.architecture {
        return Err("package architectures differ".into());
    }
    let architecture = debian_architecture(&runtime.architecture)?;
    let destination = destination.map_or_else(
        || {
            repository
                .join("dist/deb")
                .join(&runtime.version)
                .join(architecture)
        },
        Path::to_path_buf,
    );
    fs::create_dir_all(&destination)?;
    let destination = destination.canonicalize()?;
    let work = tempfile::Builder::new()
        .prefix("droidloom-deb-")
        .tempdir_in(&destination)?;
    stage(
        &runtime_archive,
        RUNTIME,
        &runtime,
        &repository,
        work.path(),
        "Runs Android applications in native Wayland windows with host GPU acceleration.",
    )?;
    stage(
        &image_archive,
        IMAGE,
        &image,
        &repository,
        work.path(),
        "Contains the Android system image and the Droidloom Android components.",
    )?;
    for (package, info) in [(RUNTIME, &runtime), (IMAGE, &image)] {
        let output = destination.join(format!(
            "{package}_{}_{}.deb",
            runtime.version, architecture
        ));
        assemble(work.path(), package, &output, info.build_date)?;
        println!("{}", output.display());
    }
    println!(
        "Install with: sudo apt install ./{RUNTIME}_{}_{}.deb ./{IMAGE}_{}_{}.deb",
        runtime.version, architecture, runtime.version, architecture
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_both_package_architectures() {
        assert_eq!(debian_architecture("x86_64").unwrap(), "amd64");
        assert_eq!(debian_architecture("aarch64").unwrap(), "arm64");
        assert!(debian_architecture("riscv64").is_err());
    }

    #[test]
    fn translates_runtime_dependencies() {
        assert_eq!(
            dependency("droidloom-image=0.1.0-47").unwrap(),
            "droidloom-image (= 0.1.0-47)"
        );
        assert_eq!(dependency("glibc").unwrap(), "libc6 (>= 2.39)");
        assert_eq!(
            dependency("wayland").unwrap(),
            "libwayland-client0, libwayland-server0"
        );
        assert_eq!(dependency("polkit").unwrap(), "polkitd, pkexec");
        assert!(dependency("some-arch-only-package").is_err());
    }

    #[test]
    fn ar_archive_starts_with_debian_binary() {
        let work = tempfile::tempdir().unwrap();
        let control = work.path().join("demo-control");
        fs::create_dir_all(&control).unwrap();
        fs::write(control.join("control"), "Package: demo\nVersion: 1\n").unwrap();
        let root = work.path().join("demo-root");
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        fs::write(root.join("usr/bin/demo"), "#!/bin/sh\n").unwrap();
        let output = work.path().join("demo.deb");
        assemble(work.path(), "demo", &output, 1).unwrap();
        let listing = Command::new("ar").arg("t").arg(&output).output().unwrap();
        assert!(listing.status.success());
        assert_eq!(
            String::from_utf8_lossy(&listing.stdout).lines().next(),
            Some("debian-binary")
        );
    }

    #[test]
    fn parses_pkginfo() {
        let info = PacmanInfo::parse(
            "pkgname = droidloom-runtime\npkgver = 0.1.0-47\narch = x86_64\npkgdesc = hello\nsize = 1234\nbuilddate = 99\ndepend = glibc\noptdepend = wireplumber: audio\n",
        )
        .unwrap();
        assert_eq!(info.name, "droidloom-runtime");
        assert_eq!(info.version, "0.1.0-47");
        assert_eq!(info.installed_size, 1234);
        assert_eq!(info.depends, ["glibc"]);
    }
}

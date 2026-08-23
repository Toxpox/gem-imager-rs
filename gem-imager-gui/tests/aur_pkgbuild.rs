//! The AUR PKGBUILDs are not built by CI, so nothing here fails when they drift from the tree they
//! package. Both invariants below were real breakages found while first building these packages:
//! without an LFS fetch the icons install as text stubs, and a stale `.SRCINFO` is what the AUR
//! actually shows and resolves dependencies from, so it silently misrepresents the package.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn aur_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("packaging/aur")
}

fn packages() -> Vec<(String, String, String)> {
    let mut found: Vec<_> = std::fs::read_dir(aur_dir())
        .expect("packaging/aur exists")
        .map(|e| e.expect("readable entry").path())
        // makepkg leaves src/, pkg/ and working clones next to the authored packages (see
        // packaging/aur/.gitignore), so a dir only counts as a package if it has a PKGBUILD.
        .filter(|p| p.is_dir() && p.join("PKGBUILD").is_file())
        .map(|dir| {
            let name = dir
                .file_name()
                .expect("named directory")
                .to_string_lossy()
                .into_owned();
            let pkgbuild = std::fs::read_to_string(dir.join("PKGBUILD"))
                .unwrap_or_else(|e| panic!("{name}/PKGBUILD is readable: {e}"));
            let srcinfo = std::fs::read_to_string(dir.join(".SRCINFO"))
                .unwrap_or_else(|e| panic!("{name}/.SRCINFO is readable: {e}"));
            (name, pkgbuild, srcinfo)
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no AUR packages found in packaging/aur");
    found
}

/// The assets the GUI embeds live in Git LFS. A plain `git` source only checks out pointer files,
/// which compile fine and then fail at runtime as undecodable images, so the fetch must be
/// explicit and the filters must be installed in the throwaway clone for it to take effect.
#[test]
fn every_pkgbuild_materialises_git_lfs_assets() {
    let lfs_tracked = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join(".gitattributes"),
    )
    .expect(".gitattributes is readable");
    assert!(
        lfs_tracked.contains("filter=lfs"),
        "assets are no longer stored in Git LFS; this test and the PKGBUILD steps it guards \
         should be removed rather than left asserting a stale requirement"
    );

    for (name, pkgbuild, _) in packages() {
        assert!(
            pkgbuild.contains("git lfs install --local"),
            "{name} does not install LFS filters, so `git lfs pull` silently skips checkout and \
             the package ships text stubs in place of its icons"
        );
        assert!(
            pkgbuild.contains("lfs pull"),
            "{name} never fetches LFS objects; makepkg's local mirror holds none"
        );
        assert!(
            pkgbuild.contains("git-lfs"),
            "{name} uses git-lfs during prepare() but does not declare it in makedepends"
        );
    }
}

/// `.SRCINFO` is generated, and the AUR trusts it over the PKGBUILD for metadata and dependency
/// resolution. Regenerating it is easy to forget, which leaves users installing against fields
/// that no longer describe the package.
#[test]
fn every_srcinfo_agrees_with_its_pkgbuild() {
    for (name, pkgbuild, srcinfo) in packages() {
        let declared: BTreeMap<&str, Vec<&str>> = srcinfo
            .lines()
            .filter_map(|l| l.split_once('='))
            .fold(BTreeMap::new(), |mut acc, (k, v)| {
                acc.entry(k.trim()).or_default().push(v.trim());
                acc
            });

        // Every runtime dependency the PKGBUILD lists has to survive into the generated file;
        // a missing one becomes a package that installs without the library it needs.
        let depends_block = pkgbuild
            .split_once("depends=(")
            .map(|(_, rest)| rest.split_once(')').expect("closed depends array").0)
            .expect("PKGBUILD declares depends");
        let expected: Vec<_> = depends_block
            .split_whitespace()
            .map(|d| d.trim_matches(['\'', '"']))
            .filter(|d| !d.is_empty() && !d.starts_with('#'))
            .collect();
        let listed = declared.get("depends").cloned().unwrap_or_default();
        for dep in &expected {
            assert!(
                listed.contains(dep),
                "{name}: `{dep}` is in the PKGBUILD but not in .SRCINFO; regenerate it with \
                 `makepkg --printsrcinfo > .SRCINFO`"
            );
        }

        assert_eq!(
            declared.get("pkgbase").and_then(|v| v.first()).copied(),
            Some(name.as_str()),
            "{name}: .SRCINFO describes a different package"
        );
    }
}

/// The packages clone whatever `url` names, so if the project moves and a PKGBUILD is left behind
/// it keeps building the old fork's `main` under the new project's package name: users get stale
/// software from an address nobody is maintaining, and nothing in the build fails. Pinning both to
/// the workspace `repository` makes the move a single edit that this test then enforces.
#[test]
fn every_pkgbuild_points_at_the_workspace_repository() {
    let manifest = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("Cargo.toml"),
    )
    .expect("workspace Cargo.toml is readable");
    let repository = manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("repository = "))
        .map(|v| v.trim().trim_matches('"'))
        .expect("workspace declares repository");

    for (name, pkgbuild, srcinfo) in packages() {
        let url = pkgbuild
            .lines()
            .find_map(|l| l.trim().strip_prefix("url="))
            .map(|v| v.trim().trim_matches(['"', '\'']))
            .unwrap_or_else(|| panic!("{name}: PKGBUILD declares url"));
        assert_eq!(
            url, repository,
            "{name}: PKGBUILD url does not match the workspace repository"
        );
        assert!(
            srcinfo
                .lines()
                .any(|l| l.trim() == format!("url = {repository}")),
            "{name}: .SRCINFO still records a different url; regenerate it with \
             `makepkg --printsrcinfo > .SRCINFO`"
        );
        // The clone address is what actually decides which tree is packaged; the display url
        // above can agree while the source silently keeps fetching the old fork.
        assert!(
            srcinfo
                .lines()
                .filter(|l| l.trim().starts_with("source = "))
                .any(|l| l.contains(&format!("git+{repository}.git"))),
            "{name}: .SRCINFO source clones a repository other than {repository}"
        );
    }
}

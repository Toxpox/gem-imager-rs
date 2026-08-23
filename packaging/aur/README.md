# AUR packages

Source for the `gem-imager-cli-git` and `gem-imager-gui-git` AUR packages. Nothing in this
repository builds them; they are published by copying `PKGBUILD` and `.SRCINFO` into the
corresponding AUR git repository.

## Publishing

The AUR only accepts pushes to `master`, and it refuses a push whose commit does not carry a
`.SRCINFO`, so regenerate that file whenever the `PKGBUILD` changes:

```shell
git -c init.defaultBranch=master clone ssh://aur@aur.archlinux.org/gem-imager-gui-git.git
cp packaging/aur/gem-imager-gui-git/{PKGBUILD,.SRCINFO} gem-imager-gui-git/
cd gem-imager-gui-git
makepkg --printsrcinfo > .SRCINFO   # only needed if the PKGBUILD was edited in place
git add PKGBUILD .SRCINFO && git commit -m "..." && git push
```

`gem-imager-gui/tests/aur_pkgbuild.rs` fails when the regeneration step is skipped. A stale
`.SRCINFO` is worse than an obviously broken one: the AUR reads metadata from it, so the package
page keeps advertising the old version and the old dependency set.

Write access needs an SSH key registered on the AUR account; these are `-git` packages, so do not
push commits that only bump `pkgver`.

## Changing the upstream repository

Both packages derive `source` and the Git LFS endpoint from `url`, so moving the project to a
different host or organisation means editing the `url=` line in each `PKGBUILD` to match the
workspace `repository` in the top-level `Cargo.toml`, then regenerating `.SRCINFO`. Nothing else
here hard-codes the repository, and `every_pkgbuild_points_at_the_workspace_repository` fails if
a package is left behind when the workspace moves.

The failure mode that test exists for is quiet: a PKGBUILD still pointing at an abandoned fork
keeps building successfully, so the package installs stale software from an address nobody
maintains without anything looking broken.

## Changing the maintainer

The `# Maintainer:` comment is a convention that nothing verifies; edit it, and move the previous
maintainer to a `# Contributor:` line below it.

Two things are harder to undo:

- **Ownership on the AUR** belongs to the account that first pushed the package, not to whatever
  the comment says. It is transferred from the package's web page, either by adding a
  co-maintainer or by disowning it so another account can adopt it. Adding the new maintainer as
  a co-maintainer *before* disowning avoids the window in which an unrelated account could adopt
  the package.
- **Commit authorship in the AUR repository** comes from the global Git identity of whoever
  pushes, and is effectively permanent once pushed. If these packages should be attributed to the
  project rather than to a personal account, set `git config user.name`/`user.email` inside the
  AUR clone *before* the first commit.

## Why `-git` packages

The only tag in this repository, `0.9`, is inherited from upstream and predates the rename of
every crate from `bb-*` to `gem-*`, so a PKGBUILD built against it fetches a tree in which
`gem-imager-cli/Cargo.toml` does not exist. These packages therefore track `main`. Once a tag
describes the current tree, versioned packages can replace them: drop the `pkgver()` function,
set `pkgver` to the tag, point `source` at it with `#tag=` and rename the directories without the
`-git` suffix.

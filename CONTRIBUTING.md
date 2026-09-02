# Contributing Guide

## Development

### Dependencies

The following dependencies are required to work with the codebase.

1. [git-lfs](https://git-lfs.com/)  
  Since this is a Git LFS repository, a lot of strange errors regarding missing assets will occur if the repository is cloned without installing `git-lfs`.
2. [Rust](https://www.rust-lang.org/)  
  I personally use [rustup](https://rustup.rs/) to manage Rust versions, but development can also be done without rustup, since everything builds with stable Rust.

#### Platform Dependencies

##### Ubuntu and other Debian-based Linux distributions

```shell
make setup-debian-deps
```

##### Fedora

```shell
make setup-fedora-deps
```

### Running the Project

**Note:** When flashing hardware or running benchmarks, ensure that you are using a release build (`--release`).

**GUI**

```shell
cargo run -p gem-imager-gui
```

**CLI**

```shell
make run-cli
cargo run -p gem-imager-cli
```

### Debugging the GUI

The GUI can be run in a debug mode that exposes [comet](https://github.com/hecrj/comet), Iced’s inspector/debugger. Press `F12` in the running application to open it.

This requires the [Dioxus CLI](https://dioxuslabs.com/learn/0.6/CLI/installation/) (`dx`), since debug mode relies on its `serve` command:

```shell
cargo install dioxus-cli
make debug-gui
```

**Note:** Hot-reloading is expected to work in this mode but currently does not behave reliably. With the `dx` CLI, code changes are picked up but the application resets to the start screen, so the `App` state is not preserved. `cargo-hot` does not work at all. Debugging via comet works regardless.

## Submitting Pull Requests

This document exists to define some conventions and standards for all pull requests. These guidelines are not rigid and may be evaluated on a case-by-case basis, but contributors are encouraged to follow them.

The process takes inspiration from
[Submitting patches: the essential guide to getting your code into the kernel](https://docs.kernel.org/process/submitting-patches.html),
although the requirements here are less stringent than those of the Linux kernel.

This guide also serves as a single reference point for contributors, avoiding the need to repeatedly explain common issues with pull requests.

### Describe Your Changes

Describe the problem you are trying to solve. Whether your patch is a one-line bug fix or a 5000-line new feature, there must be a clear underlying problem that motivated the work.

Convince the reviewer that the problem is real, worth fixing, and that it makes sense to read past the first paragraph.

Once the problem is established, describe **what** you are doing to solve it, in technical detail. Explain the change in plain English so the reviewer can verify that the code behaves as intended.

### Separate Your Changes

Each logical change should be separated into its own commit or pull request.

It is acceptable to group multiple commits into a single pull request if they address a single issue (for example, a tracked issue). Keep in mind that smaller, focused pull requests are easier to review and are typically merged faster.

### Style-Check Your Changes

Before submitting, ensure that your code follows standard Rust style conventions.

Please run:

* [rustfmt](https://rust-lang.github.io/rustfmt/)
* [clippy](https://doc.rust-lang.org/clippy/)

There is no intention to deviate from standard Rust formatting or linting rules, so these tools should be considered mandatory.

### Sign Your Work – Developer’s Certificate of Origin

Signing off commits improves tracking of who authored which changes. While this practice originates from Linux kernel development and mailing-list workflows, the simple `Signed-off-by` line is useful even here—especially when preserving authorship of work that may otherwise be abandoned.

The sign-off is a single line added at the end of the commit message, certifying that you wrote the code or otherwise have the right to submit it as an open-source contribution.

#### Developer’s Certificate of Origin 1.1

By making a contribution to this project, I certify that:

1. The contribution was created in whole or in part by me, and I have the right to submit it under the open-source license indicated in the file; or
2. The contribution is based upon previous work that, to the best of my knowledge, is covered under an appropriate open-source license, and I have the right under that license to submit that work with modifications, whether created in whole or in part by me, under the same open-source license (unless I am permitted to submit under a different license), as indicated in the file; or
3. The contribution was provided directly to me by some other person who certified (a), (b), or (c), and I have not modified it.
4. I understand and agree that this project and the contribution are public, and that a record of the contribution (including all personal information I submit with it, including my sign-off) is maintained indefinitely and may be redistributed consistent with this project or the open-source license(s) involved.

To indicate agreement, add the following line to your commit message:

```text
Signed-off-by: Toxpox <toxpox@outlook.com>
```

Use a known identity—anonymous contributions are not accepted.

If you use:

```shell
git commit -s
```

the sign-off will be added automatically. Reverts should also include a sign-off; using:

```shell
git revert -s
```

will take care of this for you.

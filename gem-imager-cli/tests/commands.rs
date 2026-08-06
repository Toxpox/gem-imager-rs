//! Tests for the non-flashing subcommands dispatched by `run`.
//!
//! `generate-completion` and `list-destinations` write straight to the process
//! stdout, so these assert on reachability and absence of panics (both commands
//! index into derived clap/`Target` metadata, which is where they break) rather
//! than on captured output. `format` is deliberately not covered: it needs a
//! real block device and would destroy it.

use clap::{Parser, ValueEnum};
use gem_imager_cli::cli::Opt;

fn run_cli(args: &[&str]) {
    let opt = Opt::try_parse_from(args).expect("argv should parse");
    gem_imager_cli::run(opt);
}

/// `generate_completion` feeds the derived command tree to `clap_complete`,
/// which panics on a malformed tree (e.g. a positional after a subcommand).
/// Generating for every supported shell keeps that regression visible.
#[test]
fn generate_completion_succeeds_for_every_shell() {
    for shell in clap_complete::Shell::value_variants() {
        let shell = shell.to_possible_value().unwrap();
        run_cli(&["gem-imager-cli", "generate-completion", shell.get_name()]);
    }
}

/// Destination enumeration is read-only, but the number of drives is
/// host-dependent (a CI container may expose none), so this asserts the command
/// completes for each output mode instead of an exact listing.
#[test]
fn list_destinations_sd_runs_in_every_output_mode() {
    run_cli(&["gem-imager-cli", "list-destinations", "sd"]);
    run_cli(&["gem-imager-cli", "list-destinations", "sd", "--no-frills"]);
    run_cli(&["gem-imager-cli", "list-destinations", "sd", "--no-filter"]);
    run_cli(&[
        "gem-imager-cli",
        "list-destinations",
        "sd",
        "--no-frills",
        "--no-filter",
    ]);
}

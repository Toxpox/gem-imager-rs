use clap::{Parser, ValueEnum};
use gem_imager_cli::cli::Opt;

fn run_cli(args: &[&str]) {
    let opt = Opt::try_parse_from(args).expect("argv should parse");
    gem_imager_cli::run(opt).unwrap_or_else(|err| panic!("{args:?} should succeed: {err:#}"));
}

#[test]
fn generate_completion_succeeds_for_every_shell() {
    for shell in clap_complete::Shell::value_variants() {
        let shell = shell.to_possible_value().unwrap();
        run_cli(&["gem-imager-cli", "generate-completion", shell.get_name()]);
    }
}

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

/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0.
 */

use crate::lint_cargo_toml::{check_crate_author, check_crate_license, check_docs_rs, fix_docs_rs};
use anyhow::{bail, Context, Result};
use clap::{App, Arg, SubCommand};
use lazy_static::lazy_static;
use std::env::set_current_dir;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

mod anchor;
mod copyright;
mod lint_cargo_toml;

fn load_repo_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .with_context(|| "couldn't load repo root")?;
    Ok(PathBuf::from(String::from_utf8(output.stdout)?.trim()))
}

fn load_vcs_files() -> Result<Vec<PathBuf>> {
    let tracked_files = Command::new("git")
        .arg("ls-tree")
        .arg("-r")
        .arg("HEAD")
        .arg("--name-only")
        .current_dir(load_repo_root()?)
        .output()
        .context("couldn't load VCS tracked files")?;
    let mut output = String::from_utf8(tracked_files.stdout)?;
    let changed_files = Command::new("git")
        .arg("diff")
        .arg("--name-only")
        .output()?;
    output.push_str(std::str::from_utf8(changed_files.stdout.as_slice())?);
    let files = output
        .lines()
        .map(|line| PathBuf::from(line.trim().to_string()));
    Ok(files.collect())
}

lazy_static! {
    static ref REPO_ROOT: PathBuf = load_repo_root().unwrap();
    static ref VCS_FILES: Vec<PathBuf> = load_vcs_files().unwrap();
}

fn repo_root() -> &'static Path {
    REPO_ROOT.as_path()
}

fn main() -> Result<()> {
    set_current_dir(repo_root())?;
    let matches = clap_app().get_matches();
    if let Some(subcommand) = matches.subcommand_matches("check") {
        let all = subcommand.is_present("all");
        if subcommand.is_present("readme") || all {
            check_readmes()?;
        }
        if subcommand.is_present("cargotoml") || all {
            check_authors()?;
            check_license()?;
        }
        if subcommand.is_present("docsrs-metadata") || all {
            check_docsrs_metadata()?;
        }
        if subcommand.is_present("license") || all {
            check_license_header()?;
        }
    } else if let Some(subcommand) = matches.subcommand_matches("fix") {
        let all = subcommand.is_present("all");
        if subcommand.is_present("readme") || all {
            fix_readmes()?;
        }
        if subcommand.is_present("docsrs-metadata") || all {
            fix_docs_rs_metadata()?;
        }
    } else {
        clap_app().print_long_help().unwrap();
    }
    Ok(())
}

fn clap_app() -> App<'static, 'static> {
    App::new("smithy-rs linter")
        .subcommand(
            SubCommand::with_name("check")
                .arg(
                    Arg::with_name("readme")
                        .takes_value(false)
                        .required(false)
                        .long("readme"),
                )
                .arg(
                    Arg::with_name("cargotoml")
                        .takes_value(false)
                        .required(false)
                        .long("authors"),
                )
                .arg(
                    Arg::with_name("docsrs-metadata")
                        .takes_value(false)
                        .required(false)
                        .long("docsrs-metadata"),
                )
                .arg(
                    Arg::with_name("license")
                        .takes_value(false)
                        .required(false)
                        .long("license"),
                )
                .arg(
                    Arg::with_name("all")
                        .takes_value(false)
                        .required(false)
                        .long("all"),
                ),
        )
        .subcommand(
            SubCommand::with_name("fix")
                .arg(
                    Arg::with_name("readme")
                        .takes_value(false)
                        .required(false)
                        .long("readme"),
                )
                .arg(
                    Arg::with_name("docsrs-metadata")
                        .takes_value(false)
                        .required(false)
                        .long("docsrs-metadata"),
                )
                .arg(
                    Arg::with_name("all")
                        .takes_value(false)
                        .required(false)
                        .long("all"),
                ),
        )
}

fn ls(path: impl AsRef<Path>) -> Result<impl Iterator<Item = PathBuf>> {
    Ok(fs::read_dir(path.as_ref())
        .with_context(|| format!("failed to ls: {:?}", path.as_ref()))?
        .map(|res| res.map(|e| e.path()))
        .collect::<Result<Vec<_>, io::Error>>()?
        .into_iter())
}

fn smithy_rs_crates() -> Result<impl Iterator<Item = PathBuf>> {
    let smithy_crate_root = repo_root().join("rust-runtime");
    Ok(ls(smithy_crate_root)?.filter(|path| is_crate(path.as_path())))
}

fn is_crate(path: &Path) -> bool {
    path.is_dir() && path.join("Cargo.toml").exists()
}

fn aws_runtime_crates() -> Result<impl Iterator<Item = PathBuf>> {
    let aws_crate_root = repo_root().join("aws").join("rust-runtime");
    Ok(ls(aws_crate_root)?.filter(|path| is_crate(path.as_path())))
}

fn all_runtime_crates() -> Result<impl Iterator<Item = PathBuf>> {
    Ok(aws_runtime_crates()?.chain(smithy_rs_crates()?))
}

fn all_cargo_tomls() -> Result<impl Iterator<Item = PathBuf>> {
    Ok(all_runtime_crates()?.map(|pkg| pkg.join("Cargo.toml")))
}

fn check_authors() -> Result<()> {
    let mut failed = 0;
    for toml in all_cargo_tomls()? {
        let local_path = toml.strip_prefix(repo_root()).expect("relative to root");
        let result = check_crate_author(toml.as_path())
            .with_context(|| format!("Error in {:?}", local_path));
        if let Err(e) = result {
            failed += 1;
            eprintln!("{:?}", e);
        }
    }
    if failed > 0 {
        bail!("{} crates had incorrect crate authors", failed)
    } else {
        eprintln!("All crates had correct authorship!");
        Ok(())
    }
}

fn check_license_header() -> Result<()> {
    let mut failed = 0;
    for license in VCS_FILES.iter() {
        let result = copyright::check_copyright_header(license);
        if let Err(e) = result {
            failed += 1;
            eprintln!("{:?}", e);
        }
    }
    if failed > 0 {
        bail!("{} files missing license headers", failed)
    }
    eprintln!("All files had correct license headers");
    Ok(())
}

/// Check that all crates have correct licensing
fn check_license() -> Result<()> {
    let mut failed = 0;
    for toml in all_cargo_tomls()? {
        let local_path = toml.strip_prefix(repo_root()).expect("relative to root");
        let result = check_crate_license(toml.as_path())
            .with_context(|| format!("Error in {:?}", local_path));
        if let Err(e) = result {
            failed += 1;
            eprintln!("{:?}", e);
        }
    }
    if failed > 0 {
        bail!("{} crates had incorrect crate licenses", failed)
    } else {
        eprintln!("All crates had correct licenses!");
        Ok(())
    }
}

/// Check that all crates have correct `[package.metadata.docs.rs]` settings
fn check_docsrs_metadata() -> Result<()> {
    let mut failed = 0;
    for toml in all_cargo_tomls()? {
        let local_path = toml.strip_prefix(repo_root()).expect("relative to root");
        let result =
            check_docs_rs(toml.as_path()).with_context(|| format!("Error in {:?}", local_path));
        if let Err(e) = result {
            failed += 1;
            eprintln!("{:?}", e);
        }
    }
    if failed > 0 {
        bail!("{} crates had incorrect docsrs metadata", failed)
    } else {
        eprintln!("All crates had correct docsrs metadata!");
        Ok(())
    }
}

/// Checks that all crates have README files
fn check_readmes() -> Result<()> {
    let no_readme = all_runtime_crates()?.filter(|dir| !dir.join("README.md").exists());

    let mut failed = 0;
    for bad_crate in no_readme {
        eprintln!(
            "{:?} is missing a README",
            bad_crate
                .strip_prefix(repo_root())
                .expect("must be relative to repo root")
        );
        failed += 1;
    }
    if failed > 0 {
        bail!("{} crates were missing READMEs", failed)
    } else {
        eprintln!("All crates have READMEs!");
        Ok(())
    }
}

fn fix_readmes() -> Result<()> {
    let readmes = all_runtime_crates()?.map(|pkg| pkg.join("README.md"));
    let mut num_fixed = 0;
    for readme in readmes {
        num_fixed += fix_readme(readme)?.then(|| 1).unwrap_or_default();
    }
    if num_fixed > 0 {
        bail!("Updated {} READMEs with footer.", num_fixed);
    } else {
        eprintln!("All READMEs have correct footers");
        Ok(())
    }
}

fn fix_docs_rs_metadata() -> Result<()> {
    let cargo_tomls = all_cargo_tomls()?;
    let mut num_fixed = 0;
    for cargo_toml in cargo_tomls {
        num_fixed += fix_docs_rs(cargo_toml.as_path())
            .with_context(|| format!("{:?}", cargo_toml))?
            .then(|| 1)
            .unwrap_or_default();
    }
    if num_fixed > 0 {
        bail!("Updated {} metadata files", num_fixed);
    } else {
        eprintln!("All crates have correct metadata");
        Ok(())
    }
}

const README_FOOTER: &str = "\nThis crate is part of the [AWS SDK for Rust](https://awslabs.github.io/aws-sdk-rust/) \
and the [smithy-rs](https://github.com/awslabs/smithy-rs) code generator. In most cases, it should not be used directly.\n";

fn fix_readme(path: impl AsRef<Path>) -> Result<bool> {
    let mut contents = fs::read_to_string(path.as_ref())
        .with_context(|| format!("failure to read readme: {:?}", path.as_ref()))?;
    let updated = anchor::replace_anchor(&mut contents, &anchor::anchors("footer"), README_FOOTER)?;
    fs::write(path.as_ref(), contents)?;
    Ok(updated)
}
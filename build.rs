use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use bindgen::callbacks::{DeriveInfo, MacroParsingBehavior, ParseCallbacks};
use git2::build::RepoBuilder;
use git2::{FetchOptions, Repository};

#[derive(Debug)]
struct BindgenRules {
    ingore_macros: HashSet<String>,
}

impl ParseCallbacks for BindgenRules {
    fn will_parse_macro(&self, name: &str) -> MacroParsingBehavior {
        if self.ingore_macros.contains(name) {
            MacroParsingBehavior::Ignore
        } else {
            MacroParsingBehavior::Default
        }
    }

    fn add_derives(&self, info: &DeriveInfo<'_>) -> Vec<String> {
        match info.name {
            "raft_result" => vec!["PartialEq".to_owned(), "Eq".to_owned()],
            _ => vec![],
        }
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    build_dqlite(&out_dir);

    let dqlite = pkg_config::Config::new()
        .statik(true)
        .probe("dqlite")
        .expect("Failed to find libdqlite");

    for lib_name in dqlite.libs {
        println!("cargo:rustc-link-lib=static={lib_name}");
    }

    let bindings = bindgen::Builder::default()
        .header("dqlite-internal.h")
        .new_type_alias("raft_result")
        .constified_enum_module("raft_result_code")
        .parse_callbacks(Box::new(BindgenRules {
            ingore_macros: HashSet::new(),
        }))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .derive_default(true)
        .derive_debug(false)
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

// TODO: remove this once refactoring-for-utils branch gets merged.
fn build_dqlite(out_dir: &PathBuf) {
    let dqlite_dir = out_dir.join("dqlite");
    let dqlite_repo = "https://github.com/canonical/dqlite.git";
    let dqlite_branch = "refactoring-for-utils";

    let mut autotools = autotools::Config::new(&dqlite_dir);

    let commit_id = if !dqlite_dir.exists() {
        let mut options = FetchOptions::new();
        options.depth(1);

        let repo = RepoBuilder::new()
            .branch(dqlite_branch)
            .fetch_options(options)
            .clone(dqlite_repo, &dqlite_dir)
            .expect("internal error: cannot clone dqlite repository");

        autotools.reconf("-iv");

        repo.head().unwrap().peel_to_commit().unwrap().id()
    } else {
        let repo =
            Repository::open(&dqlite_dir).expect("internal error: cannot open dqlite repository");

        let mut remote = repo.find_remote("origin").unwrap();

        let mut options = FetchOptions::new();
        options.depth(1);

        let fetch_ref = format!("refs/heads/{dqlite_branch}:refs/heads/{dqlite_branch}");

        remote
            .fetch(&[&fetch_ref], Some(&mut options), None)
            .expect("Git fetch failed");

        let target = repo
            .find_reference(&format!("refs/heads/{dqlite_branch}"))
            .expect("internal error: cannot find head")
            .target()
            .expect("internal error: cannot find target for head");

        let commit = repo
            .find_commit(target)
            .expect("internal error: cancannot find commit");

        repo.checkout_tree(
            commit.as_object(),
            Some(git2::build::CheckoutBuilder::new().force()),
        )
        .expect("internal error: cannot checkout commit");

        target
    };
    println!("cargo:rerun-if-changed={}", commit_id);

    autotools.disable("shared", None).fast_build(true).build();

    let pkg_config_path = out_dir.join("lib/pkgconfig");
    let pkg_config_path = match env::var_os("PKG_CONFIG_PATH") {
        Some(mut path) => {
            path.push(":");
            path.push(pkg_config_path);
            path
        }
        None => OsString::from(pkg_config_path),
    };

    unsafe {
        env::set_var("PKG_CONFIG_PATH", pkg_config_path);
    }
}

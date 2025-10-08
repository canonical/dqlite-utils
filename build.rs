use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

use git2::build::RepoBuilder;
use git2::{FetchOptions, Repository};

#[derive(Debug)]
struct IgnoreMacros(HashSet<String>);

impl bindgen::callbacks::ParseCallbacks for IgnoreMacros {
    fn will_parse_macro(&self, name: &str) -> bindgen::callbacks::MacroParsingBehavior {
        if self.0.contains(name) {
            bindgen::callbacks::MacroParsingBehavior::Ignore
        } else {
            bindgen::callbacks::MacroParsingBehavior::Default
        }
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // TODO: remove this once refactoring-for-utils branch gets merged.
    let dqlite_dir = out_dir.join("dqlite");
    let dqlite_repo = "https://github.com/canonical/dqlite.git";
    let dqlite_branch = "refactoring-for-utils";

    let commit_id = if !dqlite_dir.exists() {
        let mut options = FetchOptions::new();
        options.depth(1);

        let repo = RepoBuilder::new()
            .branch(dqlite_branch)
            .fetch_options(options)
            .clone(dqlite_repo, &dqlite_dir)
            .expect("Git clone failed");

        repo.head().unwrap().peel_to_commit().unwrap().id()
    } else {
        let repo = Repository::open(&dqlite_dir).expect("Failed to open existing repository");

        let mut remote = repo.find_remote("origin").unwrap();

        let mut options = FetchOptions::new();
        options.depth(1);

        let fetch_ref = format!("refs/heads/{0}:refs/heads/{0}", dqlite_branch);

        remote
            .fetch(&[&fetch_ref], Some(&mut options), None)
            .expect("Git fetch failed");

        let target = repo
            .find_reference(&format!("refs/heads/{}", dqlite_branch))
            .expect("Failed to find branch reference")
            .target()
            .expect("Branch reference had no target OID");

        let commit = repo
            .find_commit(target)
            .expect("Failed to find target commit");

        // Force the checkout to ensure files are updated, updating the directory timestamp
        repo.checkout_tree(
            commit.as_object(),
            Some(git2::build::CheckoutBuilder::new().force()),
        )
        .expect("Failed to checkout commit");

        target
    };

    println!("cargo:rerun-if-changed={}", commit_id);

    // Create a Config struct pointing to the cloned source directory
    autotools::Config::new(&dqlite_dir)
        .reconf("-iv")
        .disable("shared", None) // Often good for vendored dependencies
        .build(); // Runs ./configure, make, and make install into $OUT_DIR

    println!("cargo:rerun-if-changed=build.rs");

    unsafe {
        let pkgconfig_path = out_dir.join("lib/pkgconfig");
        match env::var_os("PKG_CONFIG_PATH") {
            Some(mut path) => {
                path.push(":");
                path.push(pkgconfig_path);
                env::set_var("PKG_CONFIG_PATH", path);
            }
            None => {
                env::set_var("PKG_CONFIG_PATH", pkgconfig_path);
            }
        }
    }

    let dqlite = pkg_config::Config::new()
        .statik(true)
        .probe("dqlite")
        .expect("Failed to find libdqlite");

    for lib_name in dqlite.libs {
        println!("cargo:rustc-link-lib=static={lib_name}");
    }

    let bindings = bindgen::Builder::default()
        .header("dqlite-internal.h")
        .parse_callbacks(Box::new(IgnoreMacros(
            [
                "FP_INFINITE",
                "FP_NAN",
                "FP_NORMAL",
                "FP_SUBNORMAL",
                "FP_ZERO",
                "IPPORT_RESERVED",
            ]
            .into_iter()
            .map(|s| s.into())
            .collect(),
        )))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .derive_default(true)
        .derive_debug(true)
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

use crate::git::BlobId;
use crate::util::{OsStrExt, PathExt};
use std::io::BufRead;
use std::path::Path;
use tokei::LanguageType::*;

pub fn detect_language(
    repo: &git2::Repository,
    blob_oid: BlobId,
    file_name: &Path,
) -> Option<tokei::LanguageType> {
    detect_language_from_file_name(file_name).or_else(|| {
        if let Some(ext) = file_name.extension() {
            tokei::LanguageType::from_file_extension(&ext.to_str_or_panic().to_lowercase())
        } else {
            // Note: This function will be called again when doing the actual count, that could be
            // avoided introducing some ugliness in the code. However, in practice the effect is
            // small because shebang detection is done infrequently.
            let blob = blob_oid.into_object(repo);

            detect_language_from_shebang(blob.content())
        }
    })
}

fn detect_language_from_file_name(file_name: &Path) -> Option<tokei::LanguageType> {
    match file_name.to_str_or_panic().to_lowercase().as_ref() {
        "build" | "workspace" | "module" => Some(Bazel),
        "cmakelists.txt" => Some(CMake),
        "dockerfile" => Some(Dockerfile),
        "justfile" => Some(Just),
        "gnumakefile" | "makefile" => Some(Makefile),
        "meson.build" | "meson_options.txt" => Some(Meson),
        "nuget.config" | "packages.config" | "nugetdefaults.config" => Some(NuGetConfig),
        "pkgbuild" => Some(PacmanMakepkg),
        "rakefile" => Some(Rakefile),
        "sconstruct" | "sconscript" => Some(Scons),
        "snakefile" => Some(Snakemake),
        _ => None,
    }
}

fn detect_language_from_shebang(file_content: &[u8]) -> Option<tokei::LanguageType> {
    let first_line = file_content.lines().next()?.ok()?;
    let mut words = first_line.split_whitespace();
    let first_word = words.next()?;
    if first_word == "#!/usr/bin/env" {
        let second_word = words.next()?;
        return language_from_shebang_env(second_word);
    }
    for &(language, _) in tokei::LanguageType::list() {
        for &shebang in language.shebangs() {
            if first_word == shebang {
                return Some(language);
            }
        }
    }
    None
}

fn language_from_shebang_env(word: &str) -> Option<tokei::LanguageType> {
    match word {
        "bash" => Some(Bash),
        "csh" => Some(CShell),
        "crystal" => Some(Crystal),
        "cython" => Some(Cython),
        "elvish" => Some(Elvish),
        "fish" => Some(Fish),
        "groovy" => Some(Groovy),
        "just" => Some(Just),
        "ksh" => Some(Ksh),
        "python" | "python2" | "python3" => Some(Python),
        "racket" => Some(Racket),
        "raku" | "perl6" => Some(Raku),
        "ruby" => Some(Ruby),
        "sh" => Some(Sh),
        _ => None,
    }
}

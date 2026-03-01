use std::io::BufRead;
use std::path::Path;
use tokei::LanguageType;
use tokei::LanguageType::*;

pub fn detect_language(file_name: &str, file_content: &[u8]) -> Option<LanguageType> {
    detect_language_from_file_name(file_name).or_else(|| match Path::new(file_name).extension() {
        Some(ext) => LanguageType::from_file_extension(&ext.to_str().unwrap().to_lowercase()),
        None => detect_language_from_shebang(file_content),
    })
}

fn detect_language_from_file_name(file_name: &str) -> Option<LanguageType> {
    match file_name.to_lowercase().as_ref() {
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

fn detect_language_from_shebang(file_content: &[u8]) -> Option<LanguageType> {
    let first_line = file_content.lines().next()?.ok()?;
    let mut words = first_line.split_whitespace();
    let first_word = words.next()?;
    if first_word == "#!/usr/bin/env" {
        let second_word = words.next()?;
        return language_from_shebang_env(second_word);
    }
    for &(language, _) in LanguageType::list() {
        for &shebang in language.shebangs() {
            if first_word == shebang {
                return Some(language);
            }
        }
    }
    None
}

fn language_from_shebang_env(word: &str) -> Option<LanguageType> {
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

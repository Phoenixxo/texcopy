//! removes what texcopy downloaded and saved, by name, so nothing else gets touched.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

use crate::{fetch, spec::ModelSpec, ui, vlm};

/// one thing that would be removed.
pub struct Item {
    pub path: PathBuf,
    pub what: &'static str,
    pub bytes: u64,
}

/// what texcopy put in the cache and config folders. Only names it knows are listed, so a shared
/// TEXCOPY_CACHE_DIR keeps everything else.
pub fn plan(cache: Option<&Path>, config: Option<&Path>) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(cache) = cache {
        let small = ModelSpec::default();
        for name in small.files() {
            add_with_partial(&mut items, cache, name, "formula model");
        }
        for name in vlm::model_file_names() {
            add_with_partial(&mut items, &cache.join("ovisocr2"), name, "vision model");
        }
        add_helper_downloads(&mut items, &cache.join("llama.cpp"));
    }
    if let Some(config) = config {
        add(&mut items, config.join("settings.json"), "saved settings");
    }
    items
}

fn add(items: &mut Vec<Item>, path: PathBuf, what: &'static str) {
    if let Some(bytes) = size_of(&path) {
        items.push(Item { path, what, bytes });
    }
}

/// a file and its unfinished `.part` download.
fn add_with_partial(items: &mut Vec<Item>, dir: &Path, name: &str, what: &'static str) {
    add(items, dir.join(name), what);
    add(items, dir.join(format!("{name}.part")), what);
}

/// the llama.cpp release folders (`b11146`), their archives and leftovers from a stopped install.
fn add_helper_downloads(items: &mut Vec<Item>, dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_helper_download(&name) {
            add(items, entry.path(), "llama.cpp helper");
        }
    }
}

/// `b11146`, `b11146.unpacking`, `llama-b11146-bin-macos-arm64.tar.gz` and their `.part` files.
fn is_helper_download(name: &str) -> bool {
    let name = name.strip_suffix(".part").unwrap_or(name);
    let tag_dir = name.strip_suffix(".unpacking").unwrap_or(name);
    let is_tag = |s: &str| {
        s.strip_prefix('b')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    is_tag(tag_dir)
        || (name.starts_with("llama-b") && (name.ends_with(".tar.gz") || name.ends_with(".zip")))
}

/// total size of a file or folder, None if it isn't there.
fn size_of(path: &Path) -> Option<u64> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return Some(meta.len());
    }
    let mut total = 0;
    for entry in fs::read_dir(path).ok()?.flatten() {
        total += size_of(&entry.path()).unwrap_or(0);
    }
    Some(total)
}

/// delete the items, then any folder that is now empty. Returns how many bytes went.
pub fn remove(items: &[Item], cache: Option<&Path>, config: Option<&Path>) -> Result<u64> {
    let mut freed = 0;
    for item in items {
        let gone = if item.path.is_dir() {
            fs::remove_dir_all(&item.path)
        } else {
            fs::remove_file(&item.path)
        };
        match gone {
            Ok(()) => freed += item.bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => bail!("couldn't remove {}: {e}", item.path.display()),
        }
    }
    // these only go if nothing else is in them.
    if let Some(cache) = cache {
        for dir in [cache.join("ovisocr2"), cache.join("llama.cpp")] {
            let _ = fs::remove_dir(dir);
        }
        let _ = fs::remove_dir(cache);
    }
    if let Some(config) = config {
        let _ = fs::remove_dir(config);
    }
    Ok(freed)
}

/// `texcopy --uninstall`: show what would go, ask, remove it.
pub fn run(dry_run: bool, assume_yes: bool) -> Result<()> {
    let cache = fetch::cache_dir();
    let config = fetch::config_dir();
    // a helper left behind by a crashed texcopy would keep its model file open.
    vlm::reap_orphans();

    let items = plan(cache.as_deref(), config.as_deref());
    if items.is_empty() {
        eprintln!("Nothing to remove. texcopy hasn't downloaded or saved anything here.");
        return Ok(());
    }

    let total: u64 = items.iter().map(|i| i.bytes).sum();
    eprintln!("texcopy would remove:");
    for item in &items {
        eprintln!(
            "  {:>9}  {}  ({})",
            ui::human_bytes(item.bytes),
            item.path.display(),
            item.what
        );
    }
    eprintln!("  {:>9}  in total", ui::human_bytes(total));
    eprintln!();
    eprintln!(
        "The program itself, a ./models folder and anything given with --model-dir or --vlm-dir stay where they are."
    );

    if dry_run {
        eprintln!("Dry run, nothing was removed.");
        return Ok(());
    }
    if !assume_yes && !ui::interactive() {
        bail!(
            "this needs your approval, and there is no terminal to ask in. Add --yes to approve."
        );
    }
    if !ui::confirm(
        "Remove these? Quit texcopy first if its window is open.",
        assume_yes,
    )? {
        eprintln!("Nothing was removed.");
        return Ok(());
    }

    let freed = remove(&items, cache.as_deref(), config.as_deref())?;
    eprintln!(
        "Removed {} of downloads and settings.",
        ui::human_bytes(freed)
    );
    eprintln!(
        "To remove the program too: delete texcopy.app or its folder, or run `cargo uninstall texcopy`."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("texcopy-uninstall-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn helper_downloads_are_recognised_by_name() {
        for yes in [
            "b11146",
            "b11146.unpacking",
            "llama-b11146-bin-macos-arm64.tar.gz",
            "llama-b11146-bin-win-cpu-x64.zip.part",
        ] {
            assert!(is_helper_download(yes), "{yes}");
        }
        for no in [
            "build",
            "b",
            "bxyz",
            "llama.cpp",
            "notes.txt",
            "llama-server",
        ] {
            assert!(!is_helper_download(no), "{no}");
        }
    }

    #[test]
    fn only_what_texcopy_downloads_is_listed() {
        let cache = tmp("plan-cache");
        let config = tmp("plan-config");
        write(&cache.join("encoder_model.onnx"), 100);
        write(&cache.join("decoder_model.onnx.part"), 10);
        write(&cache.join("ovisocr2/model.gguf"), 1000);
        write(&cache.join("llama.cpp/b11146/bin/llama-server"), 500);
        write(&cache.join("llama.cpp/my-own-checkout/README"), 5);
        write(&cache.join("notes.txt"), 7);
        write(&cache.join("spec.json"), 7);
        write(&config.join("settings.json"), 20);

        let items = plan(Some(&cache), Some(&config));
        let names: Vec<_> = items
            .iter()
            .map(|i| i.path.strip_prefix(&cache).unwrap_or(&i.path).to_path_buf())
            .collect();
        assert!(names.contains(&PathBuf::from("encoder_model.onnx")));
        assert!(names.contains(&PathBuf::from("decoder_model.onnx.part")));
        assert!(names.contains(&PathBuf::from("ovisocr2/model.gguf")));
        assert!(names.contains(&PathBuf::from("llama.cpp/b11146")));
        assert_eq!(items.len(), 5, "{names:?}");
        let total: u64 = items.iter().map(|i| i.bytes).sum();
        assert_eq!(total, 100 + 10 + 1000 + 500 + 20);
    }

    #[test]
    fn removing_keeps_files_it_does_not_know_and_tidies_empty_folders() {
        let cache = tmp("remove-cache");
        let config = tmp("remove-config");
        write(&cache.join("encoder_model.onnx"), 100);
        write(&cache.join("ovisocr2/model.gguf"), 1000);
        write(&cache.join("llama.cpp/b11146/llama-server"), 500);
        write(&cache.join("keep-me.txt"), 3);
        write(&config.join("settings.json"), 20);

        let items = plan(Some(&cache), Some(&config));
        let freed = remove(&items, Some(&cache), Some(&config)).unwrap();
        assert_eq!(freed, 100 + 1000 + 500 + 20);
        assert!(cache.join("keep-me.txt").is_file(), "an unknown file stays");
        assert!(
            cache.is_dir(),
            "the shared folder stays while it has something in it"
        );
        assert!(!cache.join("ovisocr2").exists());
        assert!(!cache.join("llama.cpp").exists());
        assert!(!config.exists(), "an empty settings folder goes");
        assert!(
            plan(Some(&cache), Some(&config)).is_empty(),
            "a second run has nothing to do"
        );
    }

    #[test]
    fn a_folder_that_only_held_our_files_is_removed_whole() {
        let cache = tmp("remove-all");
        write(&cache.join("tokenizer.json"), 10);
        let items = plan(Some(&cache), None);
        remove(&items, Some(&cache), None).unwrap();
        assert!(!cache.exists());
    }

    #[test]
    fn a_missing_cache_and_config_means_nothing_to_remove() {
        let nowhere = std::env::temp_dir().join("texcopy-uninstall-nowhere-dir");
        assert!(plan(Some(&nowhere), Some(&nowhere)).is_empty());
        assert!(plan(None, None).is_empty());
    }
}

//! Invalidate QML's process cache after a changed plugin install, including links.
use super::*;
use std::hash::{Hash, Hasher};

fn tree_revision(path: &Path, hash: &mut impl Hasher) -> Result<()> {
    let mut entries=std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry|entry.file_name());
    for entry in entries {
        entry.file_name().hash(hash);
        let kind=entry.file_type()?;
        anyhow::ensure!(!kind.is_symlink(),"plugin source contains a symlink: {}",entry.path().display());
        if kind.is_dir(){0u8.hash(hash);tree_revision(&entry.path(),hash)?;}
        else if kind.is_file(){1u8.hash(hash);std::fs::read(entry.path())?.hash(hash);}
    }
    Ok(())
}

pub(super) fn plan(actions: &mut Vec<Action>, config: &Path, sources: &[PathBuf], uninstall: bool) -> Result<()> {
    let plugins=config.join("omarchy/plugins");
    let stamp=config.join("fbittorrent/shell-plugin.revision");
    let changed=actions.iter().any(|item|item.path.starts_with(&plugins));
    if uninstall {
        if !actions.iter().any(|item|item.kind=="remove" && stamp.starts_with(&item.path)){remove(actions,stamp);}
        if changed {command(actions,"omarchy-restart-shell",&[]);}
        return Ok(());
    }
    if sources.is_empty(){return Ok(());}
    let mut hash=std::collections::hash_map::DefaultHasher::new();
    for source in sources {source.file_name().hash(&mut hash);tree_revision(source,&mut hash)?;}
    let revision=format!("{:016x}\n",hash.finish()).into_bytes();
    if !changed && std::fs::read(&stamp).ok().as_deref()==Some(&revision){return Ok(());}
    // Remove the previous revision before restarting: failure must remain retryable even
    // when the copied files already match on the next invocation.
    remove(actions,stamp.clone());
    command(actions,"omarchy-restart-shell",&[]);
    let mut item=action("write",stamp);item.bytes=revision;actions.push(item);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_linked_code_and_repaired_copies_restart_once_after_writes() {
        let root=std::env::temp_dir().join(format!("fbt-plugin-revision-{}",std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let source=root.join("source");std::fs::create_dir(&source).unwrap();
        let file=source.join("Dashboard.qml");std::fs::write(&file,b"old UI").unwrap();
        let stamp=root.join("fbittorrent/shell-plugin.revision");
        let sources=vec![source];let mut actions=Vec::new();
        plan(&mut actions,&root,&sources,false).unwrap();
        assert_eq!(actions[0].path,Path::new("omarchy-restart-shell"));
        assert_eq!(actions.last().unwrap().path,stamp);
        std::fs::create_dir(stamp.parent().unwrap()).unwrap();
        std::fs::write(&stamp,&actions.last().unwrap().bytes).unwrap();actions.clear();
        plan(&mut actions,&root,&sources,false).unwrap();assert!(actions.is_empty());
        std::fs::write(file,b"new UI").unwrap();plan(&mut actions,&root,&sources,false).unwrap();
        assert_eq!(actions[0].kind,"remove");assert_eq!(actions[1].path,Path::new("omarchy-restart-shell"));
        assert_ne!(std::fs::read(&stamp).unwrap(),actions.last().unwrap().bytes);
        std::fs::write(&stamp,&actions.last().unwrap().bytes).unwrap();
        actions=vec![action("write",root.join("omarchy/plugins/oliver.torrents/Dashboard.qml"))];
        plan(&mut actions,&root,&sources,false).unwrap();
        assert_eq!(actions[0].kind,"write");assert_eq!(actions[2].path,Path::new("omarchy-restart-shell"));
        // A failed restart runs after removal, before a new revision is written.
        apply(&actions[1]).unwrap();actions.clear();plan(&mut actions,&root,&sources,false).unwrap();
        assert_eq!(actions[0].path,Path::new("omarchy-restart-shell"));
        std::fs::remove_dir_all(root).unwrap();
    }
}

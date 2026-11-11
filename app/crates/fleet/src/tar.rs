//! A plain tar archive, written by hand: the evidence bundle is a handful of text files, and a
//! tar of them opens everywhere without this server needing a compression library.

fn octal(value: u64, width: usize) -> Vec<u8> {
    let mut s = format!("{:0>w$o}", value, w = width - 1).into_bytes();
    s.push(0);
    s
}

/// Add one file. Names are at most 100 bytes of plain characters; anything else is refused.
pub fn add(out: &mut Vec<u8>, name: &str, data: &[u8], mtime: u64) -> Result<(), String> {
    if name.is_empty() || name.len() > 100 || name.starts_with('/') || name.contains("..") || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._- ".contains(&b)) {
        return Err(format!("\"{name}\" cannot be a file name in the bundle."));
    }
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(&octal(0o644, 8));
    h[108..116].copy_from_slice(&octal(0, 8));
    h[116..124].copy_from_slice(&octal(0, 8));
    h[124..136].copy_from_slice(&octal(data.len() as u64, 12));
    h[136..148].copy_from_slice(&octal(mtime, 12));
    h[148..156].copy_from_slice(b"        ");
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u64 = h.iter().map(|b| *b as u64).sum();
    h[148..155].copy_from_slice(&octal(sum, 7));
    h[155] = b' ';
    out.extend_from_slice(&h);
    out.extend_from_slice(data);
    out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
    Ok(())
}

pub fn finish(out: &mut Vec<u8>) {
    out.resize(out.len() + 1024, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_tar_reads_what_we_write() {
        let mut t = Vec::new();
        add(&mut t, "bundle/README.txt", b"hello\n", 1_700_000_000).unwrap();
        add(&mut t, "bundle/devices/a b/log.jsonl", &vec![b'x'; 1000], 1_700_000_000).unwrap();
        finish(&mut t);
        assert_eq!(t.len() % 512, 0);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.tar");
        std::fs::write(&p, &t).unwrap();
        let list = std::process::Command::new("tar").arg("-tf").arg(&p).output().unwrap();
        assert!(list.status.success(), "{}", String::from_utf8_lossy(&list.stderr));
        let names = String::from_utf8_lossy(&list.stdout);
        assert!(names.contains("bundle/README.txt") && names.contains("bundle/devices/a b/log.jsonl"), "{names}");
        let x = std::process::Command::new("tar").arg("-xf").arg(&p).arg("-C").arg(dir.path()).status().unwrap();
        assert!(x.success());
        assert_eq!(std::fs::read(dir.path().join("bundle/README.txt")).unwrap(), b"hello\n");
        assert_eq!(std::fs::read(dir.path().join("bundle/devices/a b/log.jsonl")).unwrap().len(), 1000);
    }

    #[test]
    fn names_that_would_escape_the_bundle_are_refused() {
        let mut t = Vec::new();
        assert!(add(&mut t, "../etc/passwd", b"", 0).is_err());
        assert!(add(&mut t, "/abs", b"", 0).is_err());
        assert!(add(&mut t, "a\nb", b"", 0).is_err());
        assert!(t.is_empty());
    }
}

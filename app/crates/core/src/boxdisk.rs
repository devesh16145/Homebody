//! Preparing the Box's disk before its first start, with nothing but this program.
//!
//! Debian's image has to be started with a few settings the Box needs (a console the helper can
//! record, no two-minute wait for a network that is not there, and one that Apple's
//! virtualisation needs to start this kernel at all). They belong on the kernel's start-up line.
//!
//! That line lives in a file inside the image's Linux filesystem, which macOS cannot read; the
//! first version of the Box edited it with Homebrew's e2fsprogs. This does it another way that
//! needs no extra software: the image's small start-up partition is in a format every system
//! reads (FAT), and holds a three-line file that only says "go and read the real configuration".
//! That file is replaced with one that starts the kernel directly with the Box's settings, and
//! falls back to the real configuration if anything about it does not fit.
//!
//! Only two things are read from the image (its partition table and one name from the Linux
//! filesystem's first block) and one small file is rewritten in place. Nothing is mounted.

use std::io::{Read, Seek, SeekFrom, Write};

const SECTOR: u64 = 512;
/// The type every start-up (EFI system) partition carries, as it is stored on disk.
const ESP_TYPE: [u8; 16] = [0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B];
/// The first line of a file this module wrote, so a second preparation knows its own work.
const MARK: &str = "# Written by Homebody";

/// Where things are in the image.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Byte offset of the start-up partition.
    pub esp: u64,
    /// The Linux partition's own identifier in the partition table (`root=PARTUUID=…`).
    pub root_part: String,
    /// The Linux filesystem's identifier (what the start-up file searches for).
    pub root_fs: String,
}

fn bad(what: &str) -> String {
    format!("This disk image is not in the form the Box expects ({what}).")
}

fn read_at<F: Read + Seek>(f: &mut F, at: u64, len: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; len];
    f.seek(SeekFrom::Start(at)).and_then(|_| f.read_exact(&mut buf)).map_err(|_| bad("it is shorter than its own tables say"))?;
    Ok(buf)
}

fn u16_at(b: &[u8], at: usize) -> u64 {
    u16::from_le_bytes([b[at], b[at + 1]]) as u64
}

fn u32_at(b: &[u8], at: usize) -> u64 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as u64
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// An identifier as partition tables store it (first three parts little-endian), as text.
fn guid_text(b: &[u8]) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{}-{}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        hex::encode(&b[8..10]),
        hex::encode(&b[10..16])
    )
}

/// An identifier as Linux filesystems store it (in reading order), as text.
fn uuid_text(b: &[u8]) -> String {
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Read the partition table: the start-up partition, and the Linux one (the largest other).
pub fn layout<F: Read + Seek>(f: &mut F) -> Result<Layout, String> {
    let head = read_at(f, SECTOR, 92)?;
    if &head[..8] != b"EFI PART" {
        return Err(bad("no partition table"));
    }
    let (first, count, size) = (u64_at(&head, 72), u32_at(&head, 80), u32_at(&head, 84));
    if !(128..=4096).contains(&size) || count == 0 || count > 256 {
        return Err(bad("an unusual partition table"));
    }
    let table = read_at(f, first * SECTOR, (count * size) as usize)?;
    let (mut esp, mut root): (Option<u64>, Option<(u64, u64, String)>) = (None, None);
    for e in table.chunks(size as usize) {
        if e[..16].iter().all(|b| *b == 0) {
            continue;
        }
        let (start, end) = (u64_at(e, 32), u64_at(e, 40));
        if end < start {
            return Err(bad("a partition that ends before it starts"));
        }
        if e[..16] == ESP_TYPE {
            esp.get_or_insert(start * SECTOR);
        } else if root.as_ref().map_or(true, |r| end - start > r.1) {
            root = Some((start * SECTOR, end - start, guid_text(&e[16..32])));
        }
    }
    let esp = esp.ok_or_else(|| bad("no start-up partition"))?;
    let (root_at, _, root_part) = root.ok_or_else(|| bad("no Linux partition"))?;
    // The Linux filesystem's first block: its mark, then its name.
    let sb = read_at(f, root_at + 1024, 0x80)?;
    if sb[0x38..0x3A] != [0x53, 0xEF] {
        return Err(bad("its largest partition is not a Linux filesystem"));
    }
    Ok(Layout { esp, root_part, root_fs: uuid_text(&sb[0x68..0x78]) })
}

/// The start-up file: start the newest kernel in /boot directly with `extra` on its line; if
/// that cannot be done, read the image's own configuration as before.
pub fn boot_config(l: &Layout, extra: &str) -> String {
    format!(
        "{MARK}: starts Debian with the settings the Box needs.\n\
         search.fs_uuid {fs} root\n\
         set prefix=($root)'/boot/grub'\n\
         insmod regexp\n\
         set kernel=\n\
         for k in /boot/vmlinuz-*; do set kernel=\"$k\"; done\n\
         if regexp --set=1:version '^/boot/vmlinuz-(.+)$' \"$kernel\"; then\n\
         \x20 linux \"$kernel\" root=PARTUUID={part} ro {extra}\n\
         \x20 initrd \"/boot/initrd.img-$version\"\n\
         \x20 boot\n\
         fi\n\
         configfile $prefix/grub.cfg\n",
        fs = l.root_fs,
        part = l.root_part,
    )
}

/// The start-up partition's own description of itself, as far as finding one file needs.
struct Fat {
    cluster: u64,
    fat: u64,
    root: u64,
    root_len: u64,
    data: u64,
}

impl Fat {
    fn open<F: Read + Seek>(f: &mut F, at: u64) -> Result<Fat, String> {
        let b = read_at(f, at, 512)?;
        let (bps, spc, reserved, fats, root_entries, fat_sectors) = (u16_at(&b, 11), b[13] as u64, u16_at(&b, 14), b[16] as u64, u16_at(&b, 17), u16_at(&b, 22));
        let total = if u16_at(&b, 19) != 0 { u16_at(&b, 19) } else { u32_at(&b, 32) };
        if b[510..512] != [0x55, 0xAA] || bps != 512 || spc == 0 || fats == 0 {
            return Err(bad("an unreadable start-up partition"));
        }
        // The 16-bit kind only: a fixed place for the top folder, 16-bit links between clusters.
        if fat_sectors == 0 || root_entries == 0 {
            return Err(bad("a start-up partition of a kind this version does not prepare"));
        }
        let fat = at + reserved * bps;
        let root = fat + fats * fat_sectors * bps;
        let root_len = root_entries * 32;
        let data = root + root_len;
        let clusters = (total * bps).saturating_sub(data - at) / (spc * bps);
        if !(4085..65525).contains(&clusters) {
            return Err(bad("a start-up partition of a kind this version does not prepare"));
        }
        Ok(Fat { cluster: spc * bps, fat, root, root_len, data })
    }

    fn cluster_at(&self, n: u64) -> u64 {
        self.data + (n - 2) * self.cluster
    }

    /// The bytes of a folder: the fixed top folder, or the clusters of any other, in order.
    /// Each chunk comes with where it is in the image, so an entry can be rewritten in place.
    fn folder<F: Read + Seek>(&self, f: &mut F, first: Option<u64>) -> Result<Vec<(u64, Vec<u8>)>, String> {
        let Some(mut n) = first else { return Ok(vec![(self.root, read_at(f, self.root, self.root_len as usize)?)]) };
        let mut out = Vec::new();
        // A folder longer than this in a start-up partition is a damaged chain, not a folder.
        while (2..0xFFF8).contains(&n) && out.len() < 64 {
            out.push((self.cluster_at(n), read_at(f, self.cluster_at(n), self.cluster as usize)?));
            n = u16_at(&read_at(f, self.fat + n * 2, 2)?, 0);
        }
        Ok(out)
    }

    /// Find `name` (in the 8+3 form, capitals, padded) in a folder: where its entry is in the
    /// image, its first cluster and its length.
    fn find<F: Read + Seek>(&self, f: &mut F, folder: Option<u64>, name: &[u8; 11]) -> Result<(u64, u64, u64), String> {
        for (at, bytes) in self.folder(f, folder)? {
            for (i, e) in bytes.chunks(32).enumerate() {
                match e[0] {
                    0 => break,
                    0xE5 => continue,
                    _ => {}
                }
                // 0x0F: part of a long name. 0x08: the partition's own label.
                if e[11] == 0x0F || e[11] & 0x08 != 0 {
                    continue;
                }
                if e[..11].eq_ignore_ascii_case(name) {
                    return Ok((at + i as u64 * 32, u16_at(e, 26), u32_at(e, 28)));
                }
            }
        }
        Err(bad("its start-up partition has no Debian start-up file"))
    }

    /// EFI/debian/grub.cfg: its entry, its first cluster, its length.
    fn boot_file<F: Read + Seek>(&self, f: &mut F) -> Result<(u64, u64, u64), String> {
        let (_, efi, _) = self.find(f, None, b"EFI        ")?;
        let (_, debian, _) = self.find(f, Some(efi), b"DEBIAN     ")?;
        let (entry, first, len) = self.find(f, Some(debian), b"GRUB    CFG")?;
        if first < 2 || len > self.cluster {
            return Err(bad("an unusual start-up file"));
        }
        Ok((entry, first, len))
    }
}

/// The start-up file as it is now.
pub fn read_boot_config<F: Read + Seek>(f: &mut F, esp: u64) -> Result<String, String> {
    let fat = Fat::open(f, esp)?;
    let (_, first, len) = fat.boot_file(f)?;
    Ok(String::from_utf8_lossy(&read_at(f, fat.cluster_at(first), len as usize)?).into_owned())
}

/// Replace the start-up file's contents in place. The file is small (one cluster) before and
/// after, so nothing else on the partition moves: its bytes and its recorded length change.
pub fn write_boot_config<F: Read + Write + Seek>(f: &mut F, esp: u64, text: &str) -> Result<(), String> {
    let fat = Fat::open(f, esp)?;
    let (entry, first, len) = fat.boot_file(f)?;
    let now = String::from_utf8_lossy(&read_at(f, fat.cluster_at(first), len as usize)?).into_owned();
    // Only a file that is the pointer Debian ships, or one written here before, is replaced.
    if !(now.starts_with(MARK) || (now.contains("search.fs_uuid") && now.contains("configfile"))) {
        return Err(bad("its start-up file is not the one Debian ships"));
    }
    if text.len() as u64 > fat.cluster {
        return Err("The Box's start-up settings are too long for this image.".into());
    }
    let mut block = text.as_bytes().to_vec();
    block.resize(fat.cluster as usize, 0);
    let io = |e: std::io::Error| format!("The Box's disk could not be written: {e}");
    f.seek(SeekFrom::Start(fat.cluster_at(first))).and_then(|_| f.write_all(&block)).map_err(io)?;
    f.seek(SeekFrom::Start(entry + 28)).and_then(|_| f.write_all(&(text.len() as u32).to_le_bytes())).map_err(io)?;
    f.flush().map_err(io)
}

/// Prepare an image file: write the start-up file and read it back.
pub fn prepare(image: &std::path::Path, extra: &str) -> Result<(), String> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(image).map_err(|e| format!("The Box's disk could not be opened: {e}"))?;
    let l = layout(&mut f)?;
    let text = boot_config(&l, extra);
    write_boot_config(&mut f, l.esp, &text)?;
    if read_boot_config(&mut f, l.esp)? != text {
        return Err("The Box's start-up settings did not stay written.".into());
    }
    f.sync_all().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const STOCK: &str = "search.fs_uuid 60f24385-5ff6-4546-b687-2146ccf36c3b root \nset prefix=($root)'/boot/grub'\nconfigfile $prefix/grub.cfg\n";

    fn entry(name: &[u8; 11], attr: u8, cluster: u16, len: u32) -> Vec<u8> {
        let mut e = vec![0u8; 32];
        e[..11].copy_from_slice(name);
        e[11] = attr;
        e[26..28].copy_from_slice(&cluster.to_le_bytes());
        e[28..32].copy_from_slice(&len.to_le_bytes());
        e
    }

    /// A small image laid out like Debian's: a start-up partition (16-bit FAT, 2 KB clusters)
    /// holding EFI/debian/grub.cfg, and a "Linux" partition that is only its first block.
    fn image(boot_file: &str) -> Vec<u8> {
        let (esp, root) = (2048u64 * 512, 20480u64 * 512);
        let mut img = vec![0u8; (root + 4096) as usize];
        // Partition table: header at sector 1, entries at sector 2.
        img[512..520].copy_from_slice(b"EFI PART");
        img[512 + 72..512 + 80].copy_from_slice(&2u64.to_le_bytes());
        img[512 + 80..512 + 84].copy_from_slice(&128u32.to_le_bytes());
        img[512 + 84..512 + 88].copy_from_slice(&128u32.to_le_bytes());
        let mut part = |i: usize, kind: &[u8; 16], id: u8, first: u64, last: u64| {
            let at = 1024 + i * 128;
            img[at..at + 16].copy_from_slice(kind);
            img[at + 16..at + 32].copy_from_slice(&[id; 16]);
            img[at + 32..at + 40].copy_from_slice(&first.to_le_bytes());
            img[at + 40..at + 48].copy_from_slice(&last.to_le_bytes());
        };
        // Debian lists the Linux partition first and the start-up partition as number 15.
        part(0, &[7; 16], 0xAB, 20480, 20480 + 40000);
        part(14, &ESP_TYPE, 0xCD, 2048, 20479);
        // The start-up partition: 4 reserved sectors, 2 tables of 16 sectors, 512 top entries.
        let b = esp as usize;
        img[b + 11..b + 13].copy_from_slice(&512u16.to_le_bytes());
        img[b + 13] = 4;
        img[b + 14..b + 16].copy_from_slice(&4u16.to_le_bytes());
        img[b + 16] = 2;
        img[b + 17..b + 19].copy_from_slice(&512u16.to_le_bytes());
        img[b + 22..b + 24].copy_from_slice(&16u16.to_le_bytes());
        img[b + 32..b + 36].copy_from_slice(&18432u32.to_le_bytes());
        img[b + 510] = 0x55;
        img[b + 511] = 0xAA;
        let fat = b + 4 * 512;
        let top = fat + 2 * 16 * 512;
        let data = top + 512 * 32;
        let cluster = |n: usize| data + (n - 2) * 2048;
        // Clusters 2 (EFI), 3 (debian), 4 (the file) each end their own chain.
        for n in 2..5 {
            img[fat + n * 2..fat + n * 2 + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        // The top folder: the partition's label, a deleted entry, a long-name part, then EFI.
        let rows = [entry(b"BOXLABEL   ", 0x08, 0, 0), { let mut e = entry(b"GRUB    CFG", 0x20, 9, 9); e[0] = 0xE5; e }, entry(b"Agrub.cfg  ", 0x0F, 0, 0), entry(b"EFI        ", 0x10, 2, 0)];
        for (i, e) in rows.iter().enumerate() {
            img[top + i * 32..top + i * 32 + 32].copy_from_slice(e);
        }
        img[cluster(2)..cluster(2) + 32].copy_from_slice(&entry(b"DEBIAN     ", 0x10, 3, 0));
        img[cluster(3)..cluster(3) + 32].copy_from_slice(&entry(b"SHIMAA64EFI", 0x20, 0, 0));
        img[cluster(3) + 32..cluster(3) + 64].copy_from_slice(&entry(b"GRUB    CFG", 0x20, 4, boot_file.len() as u32));
        img[cluster(4)..cluster(4) + boot_file.len()].copy_from_slice(boot_file.as_bytes());
        // The Linux filesystem's first block: its mark and its name.
        let sb = root as usize + 1024;
        img[sb + 0x38..sb + 0x3A].copy_from_slice(&[0x53, 0xEF]);
        img[sb + 0x68..sb + 0x78].copy_from_slice(&hex::decode("60f243855ff64546b6872146ccf36c3b").unwrap());
        img
    }

    #[test]
    fn the_image_is_read_and_its_start_up_file_replaced_in_place() {
        let mut f = Cursor::new(image(STOCK));
        let l = layout(&mut f).unwrap();
        assert_eq!(l.esp, 2048 * 512);
        assert_eq!(l.root_fs, "60f24385-5ff6-4546-b687-2146ccf36c3b");
        assert_eq!(l.root_part, "abababab-abab-abab-abab-abababababab");
        assert_eq!(read_boot_config(&mut f, l.esp).unwrap(), STOCK);

        let text = boot_config(&l, "console=hvc0 arm64.nopauth");
        assert!(text.contains("search.fs_uuid 60f24385-5ff6-4546-b687-2146ccf36c3b root"));
        assert!(text.contains("root=PARTUUID=abababab-abab-abab-abab-abababababab ro console=hvc0 arm64.nopauth\n"));
        assert!(text.trim_end().ends_with("configfile $prefix/grub.cfg"), "the image's own configuration is the fallback");
        let before = f.get_ref().clone();
        write_boot_config(&mut f, l.esp, &text).unwrap();
        assert_eq!(read_boot_config(&mut f, l.esp).unwrap(), text);
        // A second preparation replaces its own earlier work.
        let again = boot_config(&l, "console=hvc0");
        write_boot_config(&mut f, l.esp, &again).unwrap();
        assert_eq!(read_boot_config(&mut f, l.esp).unwrap(), again);
        // Nothing changed outside the file's one cluster and the four bytes of its length.
        let after = f.get_ref();
        let changed: Vec<usize> = (0..before.len()).filter(|i| before[*i] != after[*i]).collect();
        let (first, last) = (*changed.first().unwrap(), *changed.last().unwrap());
        let file_at = 2048 * 512 + 4 * 512 + 2 * 16 * 512 + 512 * 32 + 2 * 2048;
        assert!(first >= file_at - 2048 && last < file_at + 2048, "changes at {first}..{last}, file at {file_at}");
    }

    /// Against a real image, when one is named: `HOMEBODY_TEST_IMAGE=/path/to/copy.raw cargo test
    /// -p homebody-core real_image -- --ignored`. The file is changed; give it a copy.
    #[test]
    #[ignore]
    fn a_real_image_is_prepared() {
        let path = std::env::var("HOMEBODY_TEST_IMAGE").expect("HOMEBODY_TEST_IMAGE names a copy of a Debian image");
        prepare(std::path::Path::new(&path), "console=tty0 console=hvc0 arm64.nopauth").unwrap();
        let mut f = std::fs::File::open(&path).unwrap();
        let l = layout(&mut f).unwrap();
        let text = read_boot_config(&mut f, l.esp).unwrap();
        println!("{l:?}\n{text}");
        assert!(text.starts_with(MARK) && text.contains(&l.root_part));
    }

    #[test]
    fn an_image_that_is_not_debians_is_left_alone() {
        // Not the pointer Debian ships: refused, and not a byte is written.
        let mut f = Cursor::new(image("menuentry 'Something else' { linux /vmlinuz }\n"));
        let l = layout(&mut f).unwrap();
        let before = f.get_ref().clone();
        assert!(write_boot_config(&mut f, l.esp, "x").is_err());
        assert_eq!(&before, f.get_ref());
        // Settings longer than the file's one cluster are refused.
        let mut f = Cursor::new(image(STOCK));
        assert!(write_boot_config(&mut f, l.esp, &"x".repeat(2049)).is_err());
        // No partition table at all.
        assert!(layout(&mut Cursor::new(vec![0u8; 4096])).is_err());
        // A Linux partition without the filesystem's mark.
        let mut img = image(STOCK);
        img[20480 * 512 + 1024 + 0x38] = 0;
        assert!(layout(&mut Cursor::new(img)).is_err());
        // A file shorter than its tables say does not panic.
        let mut img = image(STOCK);
        img.truncate(3000 * 512);
        assert!(layout(&mut Cursor::new(img)).is_err());
    }
}

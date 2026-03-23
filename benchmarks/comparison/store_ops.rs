use std::fs;
use std::io::{Read, Write, Seek, SeekFrom};

const HEADER_SIZE: u64 = 24;
const MAGIC: &[u8; 8] = b"JADESTR\0";

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Record {
    key: i64,
    value: i64,
}

fn main() {
    let filename = "records_rs.store";
    let _ = fs::remove_file(filename);

    // Create file with header
    let mut fp = fs::OpenOptions::new()
        .read(true).write(true).create(true)
        .open(filename).unwrap();
    fp.write_all(MAGIC).unwrap();
    fp.write_all(&0i64.to_ne_bytes()).unwrap(); // count
    fp.write_all(&(std::mem::size_of::<Record>() as i64).to_ne_bytes()).unwrap(); // rec_size
    fp.flush().unwrap();

    // Insert 10000 records
    for i in 0..10000i64 {
        fp.seek(SeekFrom::Start(8)).unwrap();
        let mut buf = [0u8; 8];
        fp.read_exact(&mut buf).unwrap();
        let count = i64::from_ne_bytes(buf);

        fp.seek(SeekFrom::Start(HEADER_SIZE + count as u64 * std::mem::size_of::<Record>() as u64)).unwrap();
        let r = Record { key: i, value: i * 7 };
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(&r as *const Record as *const u8, std::mem::size_of::<Record>())
        };
        fp.write_all(bytes).unwrap();

        let new_count = count + 1;
        fp.seek(SeekFrom::Start(8)).unwrap();
        fp.write_all(&new_count.to_ne_bytes()).unwrap();
        fp.flush().unwrap();
    }

    // Query 1000 times
    let mut total: i64 = 0;
    for j in 0..1000i64 {
        fp.seek(SeekFrom::Start(8)).unwrap();
        let mut buf = [0u8; 8];
        fp.read_exact(&mut buf).unwrap();
        let count = i64::from_ne_bytes(buf);
        fp.seek(SeekFrom::Start(HEADER_SIZE)).unwrap();

        let mut r = Record::default();
        let rbuf: &mut [u8] = unsafe {
            std::slice::from_raw_parts_mut(&mut r as *mut Record as *mut u8, std::mem::size_of::<Record>())
        };
        for _ in 0..count {
            fp.read_exact(rbuf).unwrap();
            if r.key == j {
                total += r.value;
                break;
            }
        }
    }
    println!("{}", total);

    // Count
    fp.seek(SeekFrom::Start(8)).unwrap();
    let mut buf = [0u8; 8];
    fp.read_exact(&mut buf).unwrap();
    println!("{}", i64::from_ne_bytes(buf));

    drop(fp);
    let _ = fs::remove_file(filename);
}

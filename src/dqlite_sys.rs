use std::ffi::c_char;

mod dqlite_bindings {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

pub fn string_array<const N: usize>(s: &str) -> [c_char; N] {
    if s.len() >= N {
        panic!("string is too big")
    }

    let mut array = [0 as c_char; N];
    let bytes = s.as_bytes();

    for (i, byte) in bytes.iter().enumerate() {
        array[i] = *byte as c_char;
    }

    array
}

#[test]
fn test() {
    unsafe {
        let mut uv = dqlite_bindings::uv {
            dir: string_array("."),
            ..Default::default()
        };

        let mut snapshots = 0 as *mut dqlite_bindings::uvSnapshotInfo;
        let mut n_snapshots = 0usize;

        let mut segments = 0 as *mut dqlite_bindings::uvSegmentInfo;
        let mut n_segments = 0usize;

        let mut errmsg = [0; dqlite_bindings::RAFT_ERRMSG_BUF_SIZE as usize];

        let result = dqlite_bindings::UvList(
            &mut uv as *mut _,
            &mut snapshots as *mut _,
            &mut n_snapshots as *mut _,
            &mut segments as *mut _,
            &mut n_segments as *mut _,
            errmsg.as_mut_ptr(),
        );

        assert!(result == 0);
        assert!(n_snapshots == 0);
        assert!(n_segments == 0);
    }
}

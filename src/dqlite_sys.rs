use std::ffi::CString;

mod dqlite_bindings {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

#[test]
fn test() {
    unsafe {
        let dir = CString::new(".").unwrap();

        let mut snapshots = 0 as *mut dqlite_bindings::uvSnapshotInfo;
        let mut n_snapshots = 0usize;

        let mut segments = 0 as *mut dqlite_bindings::uvSegmentInfo;
        let mut n_segments = 0usize;

        let mut errmsg = [0; dqlite_bindings::RAFT_ERRMSG_BUF_SIZE as usize];

        let result = dqlite_bindings::UvList(
            dir.as_ptr(),
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

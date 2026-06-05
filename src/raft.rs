use std::{
    ffi::{CStr, CString},
    fmt::Debug,
    ptr,
};

use anyhow::{Context, Error, Result, anyhow};

use crate::sys;
use crate::sys::{RAFT_ERRMSG_BUF_SIZE, raft_configuration, raft_result, raft_role, raft_server};

#[derive(thiserror::Error)]
#[error("{}", self.as_str())]
pub struct RaftErrorStr([u8; RAFT_ERRMSG_BUF_SIZE as usize]);

impl RaftErrorStr {
    pub(crate) fn new() -> Self {
        Self([0u8; RAFT_ERRMSG_BUF_SIZE as usize])
    }

    pub fn as_str(&self) -> &str {
        CStr::from_bytes_until_nul(self.0.as_slice())
            .expect("cannot display malformed error message: unexpected NUL")
            .to_str()
            .expect("cannot display malformed error message: malformed UTF-8")
    }

    pub(crate) fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.0.as_mut_ptr() as *mut T
    }
}

impl Debug for RaftErrorStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

pub(crate) struct RaftPtr<T>(*mut T);

impl<T> RaftPtr<T> {
    pub(crate) unsafe fn new(ptr: *mut T) -> Self {
        Self(ptr)
    }

    pub(crate) fn null() -> Self {
        Self(ptr::null_mut())
    }

    #[allow(unused)]
    pub(crate) fn as_ptr(&self) -> *const T {
        self.0
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut T {
        self.0
    }

    pub(crate) unsafe fn as_mut_ref(&mut self) -> &mut *mut T {
        &mut self.0
    }

    pub(crate) unsafe fn as_slice(&self, len: usize) -> &[T] {
        if len == 0 {
            assert!(self.0.is_null());
            return &[];
        }
        assert!(!self.0.is_null());
        unsafe { std::slice::from_raw_parts(self.0, len) }
    }

    #[allow(unused)]
    pub(crate) unsafe fn as_mut_slice(&mut self, len: usize) -> &mut [T] {
        if len == 0 {
            assert!(self.0.is_null());
            return &mut [];
        }
        assert!(!self.0.is_null());
        unsafe { std::slice::from_raw_parts_mut(self.0, len) }
    }
}

impl<T> Drop for RaftPtr<T> {
    fn drop(&mut self) {
        unsafe { sys::raft_free(self.0 as *mut _) };
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RaftConfiguration {
    pub servers: Vec<RaftServer>,
}

impl RaftConfiguration {
    pub(crate) fn new(configuration: &raft_configuration) -> Result<Self> {
        let mut servers = Vec::with_capacity(configuration.n as usize);
        let raw_servers =
            unsafe { std::slice::from_raw_parts(configuration.servers, configuration.n as usize) };
        for server in raw_servers {
            servers.push(RaftServer::new(server)?);
        }
        Ok(Self { servers })
    }

    pub(crate) fn to_raw(&self) -> Result<raft_configuration> {
        let mut c = raft_configuration::default();
        unsafe { sys::configurationInit(&mut c) };

        for server in &self.servers {
            let address = CString::new(server.address.as_str())
                .with_context(|| anyhow!("cannot use {:?} as server address", server.address))?;
            let role = match server.role {
                RaftRole::Standby => raft_role::RAFT_STANDBY,
                RaftRole::Voter => raft_role::RAFT_VOTER,
                RaftRole::Spare => raft_role::RAFT_SPARE,
            } as _;
            let rc = unsafe { sys::configurationAdd(&mut c, server.id, address.as_ptr(), role) };
            if rc != raft_result::OK {
                return Err(anyhow!("cannot add server to configuration"));
            }
        }
        Ok(c)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RaftServer {
    pub id: u64,
    pub address: String,
    pub role: RaftRole,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RaftRole {
    Standby = 0,
    Voter = 1,
    Spare = 2,
}

impl TryFrom<u8> for RaftRole {
    type Error = Error;

    fn try_from(raw: u8) -> Result<Self> {
        match raw {
            0 => Ok(Self::Standby),
            1 => Ok(Self::Voter),
            2 => Ok(Self::Spare),
            _ => Err(anyhow!("cannot convert {raw} to RaftRole")),
        }
    }
}

impl RaftRole {
    pub fn as_raw(&self) -> u32 {
        match self {
            RaftRole::Standby => raft_role::RAFT_STANDBY,
            RaftRole::Voter => raft_role::RAFT_VOTER,
            RaftRole::Spare => raft_role::RAFT_SPARE,
        }
    }
}

impl RaftServer {
    pub(crate) fn new(server: &raft_server) -> Result<Self> {
        let role = match server.role as _ {
            raft_role::RAFT_STANDBY => RaftRole::Standby,
            raft_role::RAFT_VOTER => RaftRole::Voter,
            raft_role::RAFT_SPARE => RaftRole::Spare,
            _ => return Err(anyhow!("cannot convert raft role")),
        };
        Ok(Self {
            id: server.id,
            address: unsafe { CStr::from_ptr(server.address).to_str()?.to_owned() },
            role,
        })
    }
}

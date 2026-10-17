//! Minimal HTTPS POST over WinHTTP (uses the system TLS stack and proxy
//! settings, no extra dependencies). Only used by the optional AI features.

use crate::util::wide;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::*;

struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error(what: &str) -> String {
    let code = std::io::Error::last_os_error();
    format!("{what} failed: {code}")
}

/// POSTs `body` and returns (status code, response body).
pub fn post(url: &str, headers: &[(&str, String)], body: &[u8], timeout_ms: i32) -> Result<(u32, Vec<u8>), String> {
    let wurl = wide(url);
    let mut host = vec![0u16; 256];
    let mut path = vec![0u16; 2048];
    let mut uc: URL_COMPONENTS = unsafe { std::mem::zeroed() };
    uc.dwStructSize = std::mem::size_of::<URL_COMPONENTS>() as u32;
    uc.lpszHostName = host.as_mut_ptr();
    uc.dwHostNameLength = host.len() as u32;
    uc.lpszUrlPath = path.as_mut_ptr();
    uc.dwUrlPathLength = path.len() as u32;
    if unsafe { WinHttpCrackUrl(wurl.as_ptr(), 0, 0, &mut uc) } == 0 {
        return Err(format!("Not a valid URL: {url}"));
    }
    let secure = uc.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
    unsafe {
        let session = Handle(WinHttpOpen(wide("Nib/1.0").as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        WinHttpSetTimeouts(session.0, timeout_ms, timeout_ms, timeout_ms, timeout_ms);
        let conn = Handle(WinHttpConnect(session.0, host.as_ptr(), uc.nPort, 0));
        if conn.0.is_null() {
            return Err(last_error("Connecting"));
        }
        let req = Handle(WinHttpOpenRequest(conn.0, wide("POST").as_ptr(), path.as_ptr(), null(), null(), null(), if secure { WINHTTP_FLAG_SECURE } else { 0 }));
        if req.0.is_null() {
            return Err(last_error("Opening the request"));
        }
        let mut header_text = String::new();
        for (k, v) in headers {
            header_text.push_str(&format!("{k}: {v}\r\n"));
        }
        let wh = wide(&header_text);
        if !header_text.is_empty() {
            WinHttpAddRequestHeaders(req.0, wh.as_ptr(), u32::MAX, WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE);
        }
        if WinHttpSendRequest(req.0, null(), 0, body.as_ptr() as *const _, body.len() as u32, body.len() as u32, 0) == 0 {
            return Err(last_error("Sending the request"));
        }
        if WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err(last_error("Waiting for the response"));
        }
        let mut status: u32 = 0;
        let mut len = 4u32;
        WinHttpQueryHeaders(req.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, null(), &mut status as *mut u32 as *mut _, &mut len, null_mut());
        let mut out = Vec::new();
        loop {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 {
                return Err(last_error("Reading the response"));
            }
            if avail == 0 {
                break;
            }
            let start = out.len();
            out.resize(start + avail as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(req.0, out[start..].as_mut_ptr() as *mut _, avail, &mut read) == 0 {
                return Err(last_error("Reading the response"));
            }
            out.truncate(start + read as usize);
            if read == 0 {
                break;
            }
        }
        Ok((status, out))
    }
}

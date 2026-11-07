//! The elevated helper's own quarantine below `%ProgramData%`, which every
//! user may add folders to: so the folders are created with an explicit
//! owner and a protected ACL, and an existing one is only used when it is
//! the admins' and not a junction.

use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetSecurityDescriptorDacl, IsWellKnownSid, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::UI::Shell::FOLDERID_ProgramData;

use super::{known_folder, wide};
use crate::{Error, Result};

pub fn admin_quarantine_root() -> PathBuf {
    known_folder(&FOLDERID_ProgramData)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join(r"UwUSweep\quarantine")
}

/// Full control for SYSTEM and Administrators; users may read unless
/// `private`. Not inherited from `%ProgramData%`.
fn sddl(private: bool) -> &'static str {
    if private {
        "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    } else {
        "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)"
    }
}

fn refused(path: &Path) -> Error {
    Error::Failed(format!(
        "{} is not a folder owned by the administrators",
        path.display()
    ))
}

pub fn secure_dir(path: &Path, private: bool) -> Result<()> {
    let name = wide(path);
    let text = wide(sddl(private));
    // SAFETY: the descriptor and handle are freed on every path; the
    // pointers GetSecurityInfo returns point into `sd`, freed last.
    unsafe {
        let mut desc: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut desc,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let result = (|| {
            let attrs = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: desc,
                bInheritHandle: 0,
            };
            if CreateDirectoryW(name.as_ptr(), &attrs) == 0
                && GetLastError() != ERROR_ALREADY_EXISTS
            {
                return Err(std::io::Error::last_os_error().into());
            }
            // The handle pins the folder itself, not what a junction points to.
            let h = CreateFileW(
                name.as_ptr(),
                READ_CONTROL | WRITE_DAC,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            );
            if h == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error().into());
            }
            let checked = (|| {
                let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
                if GetFileInformationByHandle(h, &mut info) == 0
                    || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
                    || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                {
                    return Err(refused(path));
                }
                let mut owner = std::ptr::null_mut();
                let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
                if GetSecurityInfo(
                    h,
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION,
                    &mut owner,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut sd,
                ) != 0
                {
                    return Err(refused(path));
                }
                let admins = IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
                    || IsWellKnownSid(owner, WinLocalSystemSid) != 0;
                LocalFree(sd);
                if !admins {
                    return Err(refused(path));
                }
                // An existing folder gets the same ACL as a new one.
                let (mut present, mut defaulted) = (0, 0);
                let mut dacl = std::ptr::null_mut();
                if GetSecurityDescriptorDacl(desc, &mut present, &mut dacl, &mut defaulted) == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let err = SetSecurityInfo(
                    h,
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    dacl,
                    std::ptr::null(),
                );
                if err != 0 {
                    return Err(std::io::Error::from_raw_os_error(err as i32).into());
                }
                Ok(())
            })();
            CloseHandle(h);
            checked
        })();
        LocalFree(desc);
        result
    }
}

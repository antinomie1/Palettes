//! Checking the user's password with PAM, as a screen locker does. It
//! blocks (PAM delays failures), so call it off the UI thread.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_CONV_ERR: c_int = 19;

#[repr(C)]
struct Message {
    style: c_int,
    text: *const c_char,
}

#[repr(C)]
struct Response {
    text: *mut c_char,
    code: c_int,
}

#[repr(C)]
struct Conversation {
    talk: extern "C" fn(c_int, *mut *const Message, *mut *mut Response, *mut c_void) -> c_int,
    data: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conversation: *const Conversation,
        handle: *mut *mut c_void,
    ) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
    fn pam_strerror(handle: *mut c_void, status: c_int) -> *const c_char;
}

unsafe extern "C" {
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn strdup(text: *const c_char) -> *mut c_char;
}

/// Answers every prompt with the password `data` points to.
extern "C" fn talk(
    count: c_int,
    messages: *mut *const Message,
    responses: *mut *mut Response,
    data: *mut c_void,
) -> c_int {
    let Ok(count) = usize::try_from(count) else {
        return PAM_CONV_ERR;
    };
    // SAFETY: PAM frees the array and each answer with free(), so both come
    // from the C allocator; it passes `count` messages, and `data` is the
    // password `authenticate` keeps alive for the call.
    unsafe {
        let answers = calloc(count.max(1), std::mem::size_of::<Response>()) as *mut Response;
        if answers.is_null() {
            return PAM_CONV_ERR;
        }
        for i in 0..count {
            let message = &**messages.add(i);
            if matches!(message.style, PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON) {
                (*answers.add(i)).text = strdup(data as *const c_char);
            }
        }
        *responses = answers;
    }
    PAM_SUCCESS
}

/// The PAM service: `palettes` where `/etc/pam.d/palettes` exists, else
/// `login`, which every distribution has.
pub fn pam_service() -> &'static str {
    if std::path::Path::new("/etc/pam.d/palettes").exists() {
        "palettes"
    } else {
        "login"
    }
}

/// Checks `password` for `user` with PAM `service`: `Ok` if it is right and
/// the account may log in, else PAM's reason.
pub fn authenticate(service: &str, user: &str, password: &str) -> Result<(), String> {
    let text = |e: std::ffi::NulError| e.to_string();
    let (service, user) = (
        CString::new(service).map_err(text)?,
        CString::new(user).map_err(text)?,
    );
    let mut secret = CString::new(password).map_err(text)?.into_bytes_with_nul();
    let conversation = Conversation {
        talk,
        data: secret.as_mut_ptr() as *mut c_void,
    };
    let mut handle = std::ptr::null_mut();
    // SAFETY: every pointer outlives the transaction, which ends here.
    let result = unsafe {
        let mut status = pam_start(service.as_ptr(), user.as_ptr(), &conversation, &mut handle);
        if status == PAM_SUCCESS {
            status = pam_authenticate(handle, 0);
        }
        if status == PAM_SUCCESS {
            status = pam_acct_mgmt(handle, 0);
        }
        let reason = (status != PAM_SUCCESS).then(|| {
            let reason = pam_strerror(handle, status);
            match reason.is_null() {
                true => format!("PAM error {status}"),
                false => CStr::from_ptr(reason).to_string_lossy().into_owned(),
            }
        });
        if !handle.is_null() {
            pam_end(handle, status);
        }
        reason.map_or(Ok(()), Err)
    };
    secret.fill(0);
    result
}

/// The name of the user running this process.
pub fn user_name() -> Option<String> {
    #[repr(C)]
    struct Passwd {
        name: *const c_char,
    }
    unsafe extern "C" {
        fn getuid() -> u32;
        fn getpwuid(uid: u32) -> *const Passwd;
    }
    // SAFETY: getpwuid returns null or a record whose first field is the
    // name; it is copied before any other call could replace it.
    unsafe {
        let entry = getpwuid(getuid());
        if entry.is_null() || (*entry).name.is_null() {
            return None;
        }
        Some(CStr::from_ptr((*entry).name).to_string_lossy().into_owned())
    }
}

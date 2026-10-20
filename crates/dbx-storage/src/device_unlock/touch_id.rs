//! Optional Touch ID unlock for the DBX Vault (macOS).
//!
//! Unlike [`DeviceUnlock`](super::DeviceUnlock), which silently releases the
//! remembered key to the logged-in user, a `TouchId` item is stored in the
//! data-protection Keychain behind a `SecAccessControl` that demands the
//! *currently enrolled* biometry (`kSecAccessControlBiometryCurrentSet`) for
//! every read. The check is enforced by the Keychain (and, on Apple Silicon,
//! the Secure Enclave), not by a separate prompt in DBX, so a process that
//! skips the UI still can't obtain the key. Adding or removing a fingerprint
//! invalidates the item and the passphrase is needed again.
//!
//! The item is created `…WhenPasscodeSetThisDeviceOnly`: it never syncs to
//! iCloud, never lands in a backup, and disappears if the login passcode is
//! removed. Nothing here ever prompts except [`TouchId::unlock`] /
//! [`TouchId::load`]; capability and enrollment checks are prompt-free.
//!
//! The data-protection Keychain only works for a signed app that carries a
//! `keychain-access-groups` (or `application-identifier`) entitlement. Without
//! it every operation fails closed and [`TouchId::availability`] reports
//! [`TouchIdAvailability::MissingEntitlement`]. Other platforms always report
//! [`TouchIdAvailability::Unsupported`].
//!
//! Errors never carry key bytes, account names, vault paths or OS error text.

use std::path::Path;

#[cfg(any(target_os = "macos", test))]
use secrecy::SecretBox;
#[cfg(any(target_os = "macos", test))]
use zeroize::Zeroize;

#[cfg(any(target_os = "macos", test))]
use super::KEY_LEN;
use super::VaultKey;

/// Separate from the ordinary remembered-key service so the two items can
/// never shadow each other.
#[cfg(any(target_os = "macos", test))]
const SERVICE: &str = "dev.jrmd.dbx.vault.touchid";

/// Reason shown in the system sheet when [`TouchId::load`] is used.
pub const DEFAULT_UNLOCK_REASON: &str = "unlock your DBX Vault";

/// Whether this build, on this machine, can create and use a Touch ID item.
/// Everything except [`Available`](Self::Available) means "don't offer it".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchIdAvailability {
    Available,
    /// Not macOS, or the platform has no biometric API.
    Unsupported,
    /// No Touch ID hardware (or biometrics are disabled by policy).
    NoBiometrics,
    /// Hardware is present but no fingerprint is enrolled.
    NotEnrolled,
    /// Too many failed attempts; macOS wants the login password first.
    LockedOut,
    /// The app isn't signed with a Keychain entitlement, so a biometry-gated
    /// item can't be stored. A packaging problem, not a user setting.
    MissingEntitlement,
}

impl TouchIdAvailability {
    pub fn is_available(self) -> bool {
        self == Self::Available
    }
}

/// Result of an explicit unlock attempt. Anything other than `Unlocked` means
/// "fall back to the passphrase".
pub enum TouchIdUnlock {
    Unlocked(VaultKey),
    /// Nothing is stored for this vault.
    NotEnrolled,
    /// The user dismissed the prompt.
    Cancelled,
    /// Biometrics can't be used right now (hardware, lockout, entitlement).
    Unavailable,
    /// The stored item can no longer be used: the fingerprint set changed or
    /// the contents were unrecognisable. It was left in place (invalidated
    /// items can't be read); call [`TouchId::forget`] and re-enroll.
    Invalidated,
    /// Touch ID didn't recognise the user, or another Keychain error.
    Failed(String),
}

impl std::fmt::Debug for TouchIdUnlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unlocked(_) => "Unlocked",
            Self::NotEnrolled => "NotEnrolled",
            Self::Cancelled => "Cancelled",
            Self::Unavailable => "Unavailable",
            Self::Invalidated => "Invalidated",
            Self::Failed(_) => "Failed",
        })
    }
}

#[derive(Clone, Debug)]
pub struct TouchId {
    account: String,
}

impl TouchId {
    /// One item per vault file, mirroring [`DeviceUnlock::for_vault`](super::DeviceUnlock::for_vault).
    pub fn for_vault(vault_path: &Path) -> Self {
        Self {
            account: super::vault_account(vault_path),
        }
    }

    /// Hardware, enrollment and entitlement check. Never prompts. Cheap, but
    /// call it off the UI thread: it makes a synchronous system call.
    pub fn availability() -> TouchIdAvailability {
        imp::availability()
    }

    pub fn is_available() -> bool {
        Self::availability().is_available()
    }

    /// Whether a Touch ID item exists for this vault. Attributes only, and the
    /// query carries an `LAContext` with `interactionNotAllowed`, so it can
    /// neither prompt nor return the key.
    pub fn is_enrolled(&self) -> Result<bool, String> {
        imp::is_enrolled(&self.account)
    }

    /// Enroll: replace any previous item with one that needs biometry for
    /// every read. Touch ID availability is checked first and, when it fails,
    /// nothing is touched (an existing item stays). Once it passes, the old
    /// item is removed before the new one is added, so a failed add leaves the
    /// vault un-enrolled rather than with a stale key.
    pub fn store(&self, key: &VaultKey) -> Result<(), String> {
        imp::store(&self.account, key)
    }

    /// Prompts for Touch ID with `reason` (a lowercase verb phrase: macOS
    /// renders "DBX is trying to <reason>") and returns the key. Every call
    /// prompts; there is no reuse window.
    pub fn unlock(&self, reason: &str) -> TouchIdUnlock {
        imp::unlock(&self.account, reason)
    }

    /// [`unlock`](Self::unlock) flattened for callers that only distinguish
    /// "got a key" from "use the passphrase": cancel, not enrolled,
    /// unavailable and invalidated all yield `Ok(None)`; only unexpected
    /// Keychain failures are `Err`.
    pub fn load(&self) -> Result<Option<VaultKey>, String> {
        match self.unlock(DEFAULT_UNLOCK_REASON) {
            TouchIdUnlock::Unlocked(key) => Ok(Some(key)),
            TouchIdUnlock::NotEnrolled
            | TouchIdUnlock::Cancelled
            | TouchIdUnlock::Unavailable
            | TouchIdUnlock::Invalidated => Ok(None),
            TouchIdUnlock::Failed(message) => Err(message),
        }
    }

    /// Delete the item. Runs with `interactionNotAllowed` so it can't prompt;
    /// absent is success.
    pub fn forget(&self) -> Result<(), String> {
        imp::forget(&self.account)
    }
}

/// Copies exactly `KEY_LEN` bytes into a vault key and wipes the source.
#[cfg(any(target_os = "macos", test))]
fn key_from_bytes(mut bytes: Vec<u8>) -> Option<VaultKey> {
    let key = <[u8; KEY_LEN]>::try_from(bytes.as_slice()).ok();
    bytes.zeroize();
    key.map(|key| SecretBox::new(Box::new(key)))
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;

    use core_foundation::base::{CFAllocatorRef, CFType, CFTypeRef, TCFType};
    use core_foundation::error::CFErrorRef;
    use core_foundation::string::{CFString, CFStringRef};
    use objc2::rc::Retained;
    use objc2_foundation::NSString;
    use objc2_local_authentication::{LAContext, LAPolicy};
    use secrecy::ExposeSecret;
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};
    use security_framework::passwords::{
        AccessControlOptions, PasswordOptions, set_generic_password_options,
    };

    use super::{SERVICE, TouchIdAvailability, TouchIdUnlock, VaultKey, key_from_bytes};

    // OSStatus values from Security/SecBase.h and LocalAuthentication/LAError.h.
    const ERR_USER_CANCELED: i32 = -128;
    const ERR_ITEM_NOT_FOUND: i32 = -25300;
    const ERR_AUTH_FAILED: i32 = -25293;
    const ERR_NOT_AVAILABLE: i32 = -25291;
    const ERR_INTERACTION_NOT_ALLOWED: i32 = -25308;
    const ERR_MISSING_ENTITLEMENT: i32 = -34018;
    const LA_BIOMETRY_NOT_AVAILABLE: isize = -6;
    const LA_BIOMETRY_NOT_ENROLLED: isize = -7;
    const LA_BIOMETRY_LOCKOUT: isize = -8;

    /// How a Keychain status should be treated by the unlock flow.
    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum Status {
        Missing,
        Cancelled,
        AuthFailed,
        Unavailable,
        Other,
    }

    pub(super) fn classify(code: i32) -> Status {
        match code {
            ERR_ITEM_NOT_FOUND => Status::Missing,
            ERR_USER_CANCELED => Status::Cancelled,
            ERR_AUTH_FAILED => Status::AuthFailed,
            ERR_MISSING_ENTITLEMENT | ERR_NOT_AVAILABLE | ERR_INTERACTION_NOT_ALLOWED => {
                Status::Unavailable
            }
            _ => Status::Other,
        }
    }

    pub(super) fn classify_biometry(code: isize) -> TouchIdAvailability {
        match code {
            LA_BIOMETRY_NOT_ENROLLED => TouchIdAvailability::NotEnrolled,
            LA_BIOMETRY_LOCKOUT => TouchIdAvailability::LockedOut,
            LA_BIOMETRY_NOT_AVAILABLE => TouchIdAvailability::NoBiometrics,
            _ => TouchIdAvailability::NoBiometrics,
        }
    }

    unsafe extern "C" {
        fn SecTaskCreateFromSelf(allocator: CFAllocatorRef) -> *const c_void;
        fn SecTaskCopyValueForEntitlement(
            task: *const c_void,
            entitlement: CFStringRef,
            error: *mut CFErrorRef,
        ) -> CFTypeRef;
    }

    /// Whether this process is signed with something that grants a
    /// data-protection Keychain access group. Reads the code signature only.
    fn has_keychain_entitlement() -> bool {
        // SAFETY: SecTask* are plain Create/Copy functions; every non-null
        // result is wrapped under the create rule so it is released.
        unsafe {
            let task = SecTaskCreateFromSelf(std::ptr::null());
            if task.is_null() {
                return false;
            }
            let task = CFType::wrap_under_create_rule(task);
            [
                "keychain-access-groups",
                "com.apple.application-identifier",
                "application-identifier",
            ]
            .into_iter()
            .any(|name| {
                let name = CFString::new(name);
                let value = SecTaskCopyValueForEntitlement(
                    task.as_CFTypeRef(),
                    name.as_concrete_TypeRef(),
                    std::ptr::null_mut(),
                );
                if value.is_null() {
                    return false;
                }
                drop(CFType::wrap_under_create_rule(value));
                true
            })
        }
    }

    pub(super) fn availability() -> TouchIdAvailability {
        // SAFETY: a fresh, thread-confined LAContext used for one preflight.
        let preflight = unsafe {
            LAContext::new()
                .canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics)
        };
        if let Err(error) = preflight {
            return classify_biometry(error.code());
        }
        if has_keychain_entitlement() {
            TouchIdAvailability::Available
        } else {
            TouchIdAvailability::MissingEntitlement
        }
    }

    fn describe(operation: &str, code: i32) -> String {
        match classify(code) {
            Status::Cancelled => "Touch ID was cancelled".into(),
            Status::AuthFailed => "Touch ID didn’t recognise you".into(),
            Status::Unavailable => "Touch ID is unavailable for this build of DBX".into(),
            Status::Missing | Status::Other => {
                format!("Touch ID couldn’t {operation} the vault key")
            }
        }
    }

    fn item() -> ItemSearchOptions {
        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::generic_password())
            .service(SERVICE)
            .ignore_legacy_keychains()
            .cloud_sync(Some(false))
            .limit(1);
        search
    }

    fn query(account: &str) -> ItemSearchOptions {
        let mut search = item();
        search.account(account);
        search
    }

    /// A context that makes the Keychain fail with `errSecInteractionNotAllowed`
    /// instead of ever presenting UI.
    fn silent_context() -> Retained<LAContext> {
        // SAFETY: a fresh LAContext configured before first use.
        unsafe {
            let context = LAContext::new();
            context.setInteractionNotAllowed(true);
            context
        }
    }

    fn password_options(account: &str) -> PasswordOptions {
        let mut options = PasswordOptions::new_generic_password(SERVICE, account);
        options.use_protected_keychain();
        options.set_access_synchronized(Some(false));
        options
    }

    pub(super) fn is_enrolled(account: &str) -> Result<bool, String> {
        // Attributes only (no kSecReturnData) under a no-interaction context.
        // An item whose attributes can't be read without UI would surface as
        // errSecInteractionNotAllowed; that is treated as "present" because
        // only an existing protected item can produce it.
        let mut search = query(account);
        search
            .load_attributes(true)
            .local_authentication_context(Some(wrap(&silent_context())));
        match search.search() {
            Ok(results) => Ok(!results.is_empty()),
            Err(error) => match error.code() {
                ERR_INTERACTION_NOT_ALLOWED => Ok(true),
                code => match classify(code) {
                    Status::Missing | Status::Unavailable => Ok(false),
                    _ => Err(describe("check", code)),
                },
            },
        }
    }

    pub(super) fn store(account: &str, key: &VaultKey) -> Result<(), String> {
        let availability = availability();
        if !availability.is_available() {
            return Err(match availability {
                TouchIdAvailability::MissingEntitlement => {
                    "Touch ID is unavailable for this build of DBX".into()
                }
                TouchIdAvailability::NotEnrolled => "No fingerprint is enrolled on this Mac".into(),
                TouchIdAvailability::LockedOut => {
                    "Touch ID is locked; unlock your Mac with its password first".into()
                }
                _ => "Touch ID isn’t available on this Mac".into(),
            });
        }
        forget(account)?;
        let access = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenPasscodeSetThisDeviceOnly),
            AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
        )
        .map_err(|error| describe("protect", error.code()))?;
        let mut options = password_options(account);
        options.set_access_control(access);
        set_generic_password_options(key.expose_secret(), options)
            .map_err(|error| describe("store", error.code()))
    }

    pub(super) fn unlock(account: &str, reason: &str) -> TouchIdUnlock {
        let context = unsafe {
            let context = LAContext::new();
            context.setLocalizedReason(&NSString::from_str(reason));
            // Every read must authenticate afresh.
            context.setTouchIDAuthenticationAllowableReuseDuration(0.0);
            context
        };
        let mut search = query(account);
        search
            .load_data(true)
            .local_authentication_context(Some(wrap(&context)));
        match search.search() {
            Ok(results) => match results.into_iter().next() {
                Some(SearchResult::Data(bytes)) => match key_from_bytes(bytes) {
                    Some(key) => TouchIdUnlock::Unlocked(key),
                    None => TouchIdUnlock::Invalidated,
                },
                _ => TouchIdUnlock::Failed(describe("read", 0)),
            },
            Err(error) => match classify(error.code()) {
                Status::Cancelled => TouchIdUnlock::Cancelled,
                Status::Unavailable => TouchIdUnlock::Unavailable,
                Status::Missing => match is_enrolled(account) {
                    Ok(true) => TouchIdUnlock::Invalidated,
                    Ok(false) => TouchIdUnlock::NotEnrolled,
                    Err(message) => TouchIdUnlock::Failed(message),
                },
                Status::AuthFailed | Status::Other => {
                    TouchIdUnlock::Failed(describe("read", error.code()))
                }
            },
        }
    }

    pub(super) fn forget(account: &str) -> Result<(), String> {
        // SecItemDelete doesn't evaluate the access control; the no-interaction
        // context guarantees it can't prompt even if that ever changes.
        let mut search = query(account);
        search.local_authentication_context(Some(wrap(&silent_context())));
        match search.delete() {
            Ok(()) => Ok(()),
            Err(error) => match classify(error.code()) {
                Status::Missing | Status::Unavailable => Ok(()),
                _ => Err(describe("remove", error.code())),
            },
        }
    }

    /// An LAContext as a CF object for `kSecUseAuthenticationContext`; the
    /// Objective-C object is toll-free for retain/release.
    fn wrap(context: &Retained<LAContext>) -> CFType {
        // SAFETY: the pointer is a live NSObject; wrap_under_get_rule retains it.
        unsafe { CFType::wrap_under_get_rule(Retained::as_ptr(context).cast::<c_void>()) }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn statuses_map_to_fallbacks() {
            assert_eq!(classify(-128), Status::Cancelled);
            assert_eq!(classify(-25300), Status::Missing);
            assert_eq!(classify(-25293), Status::AuthFailed);
            assert_eq!(classify(-34018), Status::Unavailable);
            assert_eq!(classify(-1), Status::Other);
        }

        #[test]
        fn biometry_errors_map_to_availability() {
            assert_eq!(classify_biometry(-7), TouchIdAvailability::NotEnrolled);
            assert_eq!(classify_biometry(-8), TouchIdAvailability::LockedOut);
            assert_eq!(classify_biometry(-6), TouchIdAvailability::NoBiometrics);
            assert_eq!(classify_biometry(-1), TouchIdAvailability::NoBiometrics);
        }

        #[test]
        fn messages_carry_no_os_text() {
            for code in [-128, -25300, -25293, -34018, -1] {
                assert!(!describe("read", code).contains(&code.to_string()));
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{TouchIdAvailability, TouchIdUnlock, VaultKey};

    pub(super) fn availability() -> TouchIdAvailability {
        TouchIdAvailability::Unsupported
    }

    pub(super) fn is_enrolled(_account: &str) -> Result<bool, String> {
        Ok(false)
    }

    pub(super) fn store(_account: &str, _key: &VaultKey) -> Result<(), String> {
        Err("Touch ID is only available on macOS".into())
    }

    pub(super) fn unlock(_account: &str, _reason: &str) -> TouchIdUnlock {
        TouchIdUnlock::Unavailable
    }

    pub(super) fn forget(_account: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_must_be_exactly_the_vault_key_length() {
        assert!(key_from_bytes(vec![7; KEY_LEN]).is_some());
        assert!(key_from_bytes(vec![7; KEY_LEN - 1]).is_none());
        assert!(key_from_bytes(vec![7; KEY_LEN + 1]).is_none());
        assert!(key_from_bytes(Vec::new()).is_none());
    }

    #[test]
    fn debug_output_never_includes_the_key() {
        let unlocked = TouchIdUnlock::Unlocked(SecretBox::new(Box::new([0x5a; KEY_LEN])));
        assert_eq!(format!("{unlocked:?}"), "Unlocked");
        let touch_id = TouchId::for_vault(Path::new("/tmp/vault.dbx"));
        assert!(!format!("{touch_id:?}").contains("secret"));
    }

    #[test]
    fn items_do_not_share_the_ordinary_service() {
        assert_ne!(SERVICE, super::super::SERVICE);
    }

    #[test]
    fn availability_only_available_when_everything_works() {
        assert!(TouchIdAvailability::Available.is_available());
        for blocked in [
            TouchIdAvailability::Unsupported,
            TouchIdAvailability::NoBiometrics,
            TouchIdAvailability::NotEnrolled,
            TouchIdAvailability::LockedOut,
            TouchIdAvailability::MissingEntitlement,
        ] {
            assert!(!blocked.is_available());
        }
    }
}

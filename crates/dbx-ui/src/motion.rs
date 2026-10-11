//! Motion preferences shared by DBX and GPUI's animation machinery.

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionPreference {
    #[default]
    System,
    Reduce,
    Full,
}

impl MotionPreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Reduce, Self::Full];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Reduce => "Reduce",
            Self::Full => "Full",
        }
    }

    fn resolve(self, system: Option<bool>) -> bool {
        match self {
            // Use static presentation until the OS responds, and on desktops
            // that cannot report the setting. Full is an explicit opt-in.
            Self::System => system.unwrap_or(true),
            Self::Reduce => true,
            Self::Full => false,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct MotionState {
    preference: MotionPreference,
    system: Option<bool>,
}
impl Global for MotionState {}

pub fn preference(cx: &App) -> MotionPreference {
    cx.try_global::<MotionState>()
        .map_or(MotionPreference::System, |state| state.preference)
}

pub fn set_preference(preference: MotionPreference, cx: &mut App) {
    let mut state = cx.try_global::<MotionState>().copied().unwrap_or_default();
    state.preference = preference;
    apply(state, cx);
}

fn apply(state: MotionState, cx: &mut App) {
    cx.set_global(state);
    cx.set_reduce_motion(state.preference.resolve(state.system));
}

pub fn init(preference: MotionPreference, cx: &mut App) {
    set_preference(preference, cx);
    cx.spawn(async move |cx| {
        loop {
            // Portal I/O stays off the UI thread. AppKit is read on the main
            // thread; the Windows preference call is also immediate.
            #[cfg(target_os = "linux")]
            let system = cx
                .background_executor()
                .spawn(async { system_preference() })
                .await;
            #[cfg(not(target_os = "linux"))]
            let system = cx.update(|_| system_preference());
            cx.update(|cx| {
                let mut state = *cx.global::<MotionState>();
                state.system = system;
                apply(state, cx);
            });
            cx.background_executor()
                .timer(std::time::Duration::from_secs(15))
                .await;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

#[cfg(target_os = "linux")]
fn system_preference() -> Option<bool> {
    use zbus::{blocking::connection::Builder, zvariant::OwnedValue};
    let connection = Builder::session()
        .ok()?
        .method_timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )
    .ok()?;
    let value: OwnedValue = proxy
        .call("ReadOne", &("org.freedesktop.appearance", "reduced-motion"))
        .ok()?;
    // The portal specifies that unknown enum values mean no preference.
    u32::try_from(value).ok().map(|value| value == 1)
}

#[cfg(target_os = "macos")]
fn system_preference() -> Option<bool> {
    Some(objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
}

#[cfg(target_os = "windows")]
fn system_preference() -> Option<bool> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: i32 = 0;
    // SAFETY: this query writes one BOOL to the live, correctly sized local.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    (success != 0).then_some(enabled == 0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn system_preference() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn os_updates_respect_explicit_override_and_returning_to_system(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            apply(
                MotionState {
                    preference: MotionPreference::System,
                    system: Some(true),
                },
                cx,
            );
            assert!(cx.reduce_motion());
            set_preference(MotionPreference::Full, cx);
            apply(
                MotionState {
                    preference: preference(cx),
                    system: Some(true),
                },
                cx,
            );
            assert!(!cx.reduce_motion());
            set_preference(MotionPreference::System, cx);
            assert!(cx.reduce_motion());
            apply(
                MotionState {
                    preference: preference(cx),
                    system: Some(false),
                },
                cx,
            );
            assert!(!cx.reduce_motion());
            set_preference(MotionPreference::Reduce, cx);
            assert!(cx.reduce_motion());
        });
    }

    #[test]
    fn unavailable_os_prefers_static_presentation() {
        assert!(MotionPreference::System.resolve(None));
        assert!(!MotionPreference::Full.resolve(None));
    }
}

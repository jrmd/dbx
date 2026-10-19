//! Explicit registration and selection of community driver manifests.
use super::*;
use dbx_core::driver_plugin::DriverManifest;
fn directory() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dbx/drivers")
}
fn registered() -> Vec<(PathBuf, DriverManifest)> {
    let Ok(entries) = std::fs::read_dir(directory()) else {
        return Vec::new();
    };
    let mut manifests = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let manifest = DriverManifest::read(&path).ok()?;
            Some((path, manifest))
        })
        .collect::<Vec<_>>();
    manifests.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    manifests
}
impl DbxApp {
    fn register_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Register a trusted DBX driver manifest".into()),
        });
        cx.spawn_in(window,async move |this,cx| {
            let paths=match receiver.await {Ok(Ok(Some(paths)))=>paths,_=>return Ok::<(),anyhow::Error>(())};
            let Some(source)=paths.first() else {return Ok(());};
            let result=(||->Result<(),String> {
                let manifest=DriverManifest::read(source).map_err(|error|error.to_string())?;
                if !manifest.executable.is_file() {return Err("The manifest executable does not exist".into());}
                let path=directory().join(format!("{}.json",manifest.id));
                if path.exists() {return Err("A driver with this id is already registered. Remove it from the drivers directory before replacing it.".into());}
                std::fs::create_dir_all(directory()).map_err(|error|error.to_string())?;
                let bytes=serde_json::to_vec_pretty(&manifest).map_err(|error|error.to_string())?;
                std::fs::write(path,bytes).map_err(|error|error.to_string())?;Ok(())
            })();
            this.update_in(cx,|this,_,cx|match result {Ok(())=>this.show_toast(ToastKind::Success,"Driver registered. Select it to create a connection.",cx),Err(error)=>this.show_toast(ToastKind::Error,error,cx)})?;
            Ok(())
        }).detach();
    }
    fn select_driver(&mut self, path: PathBuf, manifest: DriverManifest, cx: &mut Context<Self>) {
        self.select_kind(manifest.family, cx);
        self.draft.driver_manifest = Some(path);
        self.draft.mode = ConnectionFormMode::ConnectionString;
        self.draft.connection_name.update(cx, |value, cx| {
            *value = manifest.name;
            cx.notify();
        });
        self.draft.connection_url.update(cx, |value, cx| {
            *value = format!("{}://localhost/database", manifest.schemes[0]);
            cx.notify();
        });
        cx.notify();
    }
    pub(super) fn render_driver_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        div().flex().flex_col().gap(crate::theme::ui_px(8.))
            .child(div().flex().items_center().justify_between().child("Community drivers")
                .child(button("register-driver","Register trusted driver…",ButtonKind::Quiet).on_click(cx.listener(|this,_,window,cx|this.register_driver(window,cx)))))
            .child(div().text_size(crate::theme::text_px(11.)).text_color(theme().text_muted).child("Driver programs run locally with the credentials of connections you open. Register manifests only from authors you trust."))
            .child(div().flex().flex_wrap().gap(crate::theme::ui_px(8.)).children(registered().into_iter().map(|(path,manifest)| {
                button(SharedString::from(format!("driver-{}",manifest.id)),manifest.name.clone(),ButtonKind::Quiet)
                    .on_click(cx.listener(move |this,_,_,cx|this.select_driver(path.clone(),manifest.clone(),cx)))
            }))).into_any_element()
    }
}

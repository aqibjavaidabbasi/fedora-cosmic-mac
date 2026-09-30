#!/usr/bin/env python3
"""Patch cosmic-osd so the system dialogs look like macOS alerts (run from its source root).
Exits 1 if the code no longer matches.

- Log Out / Restart / Shut Down confirmations and the password prompt (polkit)
  use patches/mac_alert.rs: a small frosted card, icon on top, bold centered
  title, short message, capsule buttons with the default one in blue.
- The headphones/headset chooser keeps COSMIC's layout.
"""
import os, re, shutil, sys

here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mac_alert.rs"), "src/mac_alert.rs")

def patch(path, edits):
    s = open(path).read()
    for pat, rep in edits:
        s, n = re.subn(pat, rep, s, count=1, flags=re.S)
        if n != 1:
            print(f"osd patch failed in {path}: {pat[:70]}", file=sys.stderr)
            sys.exit(1)
    open(path, "w").write(s)

patch("src/main.rs", [(r"\nmod components;\n", "\nmod components;\nmod mac_alert;\n")])

patch("src/components/app.rs", [
    (r"(            let countdown = &countdown\.to_string\(\);\n)",
     r"""\1            if !matches!(cur_action, OsdTask::ConfirmHeadphones { .. }) {
                let glyph = match cur_action {
                    OsdTask::LogOut => "system-log-out-symbolic",
                    OsdTask::Restart | OsdTask::EnterBios => "system-restart-symbolic",
                    _ => "system-shutdown-symbolic",
                };
                let extra = matches!(cur_action, OsdTask::Shutdown)
                    .then(|| crate::mac_alert::Action::new(fl!("restart"), Some(Msg::Action(OsdTask::Restart))));
                let card = crate::mac_alert::alert(
                    text_icon(glyph, 56).into(),
                    title.clone(),
                    fl!("confirm-body", HashMap::from_iter(vec![("action", action), ("countdown", countdown)])),
                    None,
                    crate::mac_alert::Action::new(fl!("cancel"), Some(Msg::Cancel)).id(CANCEL_ID.clone()),
                    crate::mac_alert::Action::new(fl!("confirm", HashMap::from_iter(vec![("action", action)])), Some(Msg::Confirm))
                        .id(CONFIRM_ID.clone()),
                    extra,
                );
                return Element::from(
                    autosize::autosize(card, AUTOSIZE_DIALOG_ID.clone())
                        .limits(Limits::NONE.min_width(1.).min_height(1.).max_width(900.).max_height(900.)),
                );
            }
"""),
])

patch("src/components/polkit_dialog.rs", [
    (r"        widget::autosize::autosize\(\n            widget::dialog::dialog\(\)\n.*?\.secondary_action\(cancel_button\),\n",
     r"""        let _ = (cancel_button, authenticate_button);
        let can_submit = self.sensitive && self.responder.is_some();
        widget::autosize::autosize(
            crate::mac_alert::alert(
                icon.into(),
                self.msg_authentication_required.clone(),
                self.params.message.clone(),
                Some(widget::column::with_children(right_column).spacing(4).into()),
                crate::mac_alert::Action::new(self.msg_cancel.clone(), self.sensitive.then_some(Msg::Cancel)),
                crate::mac_alert::Action::new(self.msg_authenticate.clone(), can_submit.then_some(Msg::Authenticate)),
                None,
            ),
"""),
])

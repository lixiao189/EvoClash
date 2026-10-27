use super::*;

pub(super) fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            _ => c,
        })
        .collect()
}

pub(super) fn esc(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(';', "\\;")
        .replace('\n', " ")
}

pub(super) fn unesc(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn fmt_input(input: &PlayerInput) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {}",
        input.move_x,
        input.move_y,
        input.aim.x.to_bits(),
        input.aim.y.to_bits(),
        u8::from(input.fire),
        u8::from(input.dash),
        u8::from(input.interact),
        u8::from(input.throw),
        u8::from(input.skill_a),
        u8::from(input.skill_b),
        input.select,
    )
}

pub fn fmt_command(cmd: &PlayerCommand) -> String {
    match cmd {
        PlayerCommand::Spend(perk) => format!("spend {perk:?}"),
        PlayerCommand::Refund(perk) => format!("refund {perk:?}"),
        PlayerCommand::ResetPerks => "reset".into(),
        PlayerCommand::Buy(item) => format!("buy {item:?}"),
        PlayerCommand::Unbuy(item) => format!("unbuy {item:?}"),
        PlayerCommand::Deploy => "deploy".into(),
        PlayerCommand::DeployLast => "deploy_last".into(),
    }
}

pub fn fmt_room(cmd: &RoomCommand) -> String {
    match cmd {
        RoomCommand::SetSettings { bots, monsters } => format!(
            "settings {} {}",
            bots.map(|n| n.to_string()).unwrap_or_else(|| "off".into()),
            u8::from(*monsters)
        ),
        RoomCommand::Start => "start".into(),
        RoomCommand::Cancel => "cancel".into(),
        RoomCommand::Leave => "leave".into(),
        RoomCommand::PauseAutoStart => "pause_auto".into(),
        RoomCommand::ResumeAutoStart => "resume_auto".into(),
        RoomCommand::SpectateNext => "spectate_next".into(),
        RoomCommand::SpectatePrev => "spectate_prev".into(),
    }
}

pub(super) fn settings_line(settings: &RoomSettings) -> String {
    let bots = settings
        .bots
        .map(|n| n.to_string())
        .unwrap_or_else(|| "off".into());
    let code = if settings.code.is_empty() {
        "-".to_string()
    } else {
        settings.code.clone()
    };
    format!(
        "settings {bots} {} {} {} {code} {}",
        u8::from(settings.monsters),
        settings.host,
        u8::from(settings.private_room),
        u8::from(settings.auto_paused),
    )
}

pub fn phase_name(phase: Phase) -> &'static str {
    crate::gameplay::phase_name(phase)
}

pub(super) fn split_esc(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                cur.push(next);
            }
        } else if c == '|' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

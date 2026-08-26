//! HTML formatting helpers. Privacy: display name / username only.

/// Escape text for Telegram HTML parse mode.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Public-facing label — never email / UUID / wallet.
pub fn public_name(display_name: Option<&str>, username: Option<&str>) -> String {
    display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| username.map(str::trim).filter(|s| !s.is_empty()))
        .unwrap_or("Player")
        .to_owned()
}

pub fn format_usd(micro: i64) -> String {
    let dollars = micro as f64 / 1_000_000.0;
    if dollars.fract() == 0.0 {
        format!("${dollars:.0}")
    } else {
        format!("${dollars:.2}")
    }
}

/// PnL for leaderboard: `0`, `+$1.25`, or `-$0.50`.
pub fn format_pnl(micro: i64) -> String {
    if micro == 0 {
        "0".into()
    } else if micro > 0 {
        format!("+{}", format_usd(micro))
    } else {
        format!("-{}", format_usd(-micro))
    }
}

pub fn ordinal(rank: usize) -> String {
    let n = rank.max(1);
    let suffix = match n % 100 {
        11 | 12 | 13 => "th",
        _ => match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };
    format!("{n}{suffix}")
}

pub fn lobby_created_html(
    lobby_name: &str,
    game_name: &str,
    creator: &str,
    entry_micro: i64,
    is_sponsored: bool,
    max_players: u8,
) -> String {
    let lobby_type = if entry_micro <= 0 {
        "Free".to_owned()
    } else if is_sponsored {
        format!("Sponsored · {}", format_usd(entry_micro))
    } else {
        format!("Paid · {} entry", format_usd(entry_micro))
    };

    // Lobby name stays bold (not a link). Preview URL is set via
    // `link_preview_options` on send; Join lobby button carries the same URL.
    format!(
        "🎮 <b>{}</b>\n\n\
         <b>Game:</b> {}\n\
         <b>Host:</b> {}\n\
         <b>Type:</b> {}\n\
         <b>Players:</b> 1/{}",
        esc(lobby_name),
        esc(game_name),
        esc(creator),
        esc(&lobby_type),
        max_players,
    )
}

/// One standing row: rank, public name, prize micro (0 = no payout shown as —).
pub struct StandingRow {
    pub rank: usize,
    pub name: String,
    pub prize_micro: i64,
}

pub fn lobby_finished_html(
    lobby_name: &str,
    game_name: &str,
    pot_micro: i64,
    standings: &[StandingRow],
) -> String {
    let mut standings_block = String::new();
    for row in standings {
        let prize = if row.prize_micro > 0 {
            format!(" · +{}", format_usd(row.prize_micro))
        } else {
            String::new()
        };
        standings_block.push_str(&format!(
            "\n{} · {}{}",
            esc(&ordinal(row.rank)),
            esc(&row.name),
            prize
        ));
    }
    if standings_block.is_empty() {
        standings_block.push_str("\n—");
    }

    let pot_line = if pot_micro > 0 {
        format!("\n<b>Pot:</b> {}", esc(&format_usd(pot_micro)))
    } else {
        String::new()
    };

    format!(
        "🏁 <b>Match complete</b>\n\n\
         <b>Lobby:</b> {}\n\
         <b>Game:</b> {}{}\n\
         <b>Standings:</b>{}",
        esc(lobby_name),
        esc(game_name),
        pot_line,
        standings_block,
    )
}

pub fn lobby_cancelled_html(lobby_name: &str, game_name: &str) -> String {
    format!(
        "🗑 <b>Lobby closed</b>\n\n\
         <b>Lobby:</b> {}\n\
         <b>Game:</b> {}\n\n\
         This lobby was cancelled before the match started.",
        esc(lobby_name),
        esc(game_name),
    )
}

pub fn leaderboard_html(season_label: &str, rows: &[(u32, String, i64, i32, i32, i64)]) -> String {
    if rows.is_empty() {
        return format!(
            "🏆 <b>Leaderboard</b> · {}\n\nNo ranked players yet.",
            esc(season_label)
        );
    }
    let mut body = format!("🏆 <b>Top 10</b> · {}\n\n", esc(season_label));
    for (rank, name, points, wins, matches, pnl) in rows {
        body.push_str(&format!(
            "<b>{}.</b> {}\n   <code>{}</code> pts · {}W / {} · PnL {}\n",
            rank,
            esc(name),
            points,
            wins,
            matches,
            esc(&format_pnl(*pnl))
        ));
    }
    body
}

pub fn join_keyboard(room_url: &str) -> serde_json::Value {
    serde_json::json!({
        "inline_keyboard": [[
            { "text": "Join lobby", "url": room_url }
        ]]
    })
}

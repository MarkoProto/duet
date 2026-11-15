//! The note two people see after both said yes: what they did together, and what is next.
//!
//! Written from the room's own record of the activity, never from the chat, which the lobby does
//! not have. The rules always write one. A model may write a warmer one, and it is used only if
//! every number in it is a number from the record, in both languages.

use duet_room::Bi;
use duet_room::text::clean_text;
use serde_json::{Value, json};

use crate::planner::{safe, title};

const MAX_CHARS: usize = 160;

/// What a note may say, as plain facts. Numbers the activity's page sent are capped, since a page
/// could send anything.
pub fn facts(kind: &str, summary: &Value, secs: u64, next_mins: u64) -> Value {
    let n = |key: &str| summary.get(key).and_then(Value::as_u64).map(|v| v.min(999));
    // A count of matches is never more than the rounds it was counted in.
    let within = |key: &str| n(key).map(|v| v.min(n("rounds").unwrap_or(0)));
    let record = match kind {
        "wavelength" => json!({"rounds": n("rounds"), "in_sync": within("synced")}),
        "sketch" => json!({"strokes": n("strokes")}),
        "signal" => json!({"paths_crossed": n("levels"), "slips": n("slips")}),
        "thisorthat" => json!({"rounds": n("rounds"), "same_pick": within("same")}),
        "story" => json!({"sentences": n("lines")}),
        _ => json!({}),
    };
    let record: serde_json::Map<String, Value> = record
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let t = title(kind);
    json!({
        "activity": kind,
        "activity_name": {"en": t.en, "ja": t.ja},
        "record": record,
        "minutes_together": secs.div_ceil(60).max(1),
        "next_time_minutes": next_mins,
        "next_time_activity": "a different one, chosen for them",
    })
}

/// "1 round", "3 rounds".
fn count(n: u64, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The note the rules write.
pub fn rules(f: &Value) -> Bi {
    let r = &f["record"];
    let mins = f["next_time_minutes"].as_u64().unwrap_or(45);
    let num = |k: &str| r.get(k).and_then(Value::as_u64);
    let name = |l: &str| f["activity_name"][l].as_str().unwrap_or("").to_string();
    let (en, ja) = match (
        f["activity"].as_str().unwrap_or(""),
        num("rounds"),
        num("in_sync"),
        num("strokes"),
        num("paths_crossed"),
    ) {
        ("wavelength", Some(rounds), Some(sync), _, _) if rounds > 0 => {
            let of = count(rounds, "round");
            (
                match sync {
                    0 => format!("You two played {of} of Same Wavelength."),
                    1 => format!("You two were in sync once in {of} of Same Wavelength."),
                    _ => format!("You two were in sync {sync} times in {of} of Same Wavelength."),
                },
                format!("以心伝心で{rounds}問中{sync}回、気持ちが一致しました。"),
            )
        }
        ("sketch", _, _, Some(strokes), _) if strokes > 0 => (
            format!(
                "You drew one picture together, {} between you.",
                count(strokes, "stroke")
            ),
            format!("二人で一枚の絵を描きました。線は全部で{strokes}本。"),
        ),
        ("signal", _, _, _, Some(paths)) if paths > 0 => (
            format!("You crossed {} together in Signal.", count(paths, "path")),
            format!("シグナルで{paths}本の道を一緒に渡りました。"),
        ),
        ("thisorthat", Some(rounds), _, _, _) if rounds > 0 && num("same_pick").is_none() => (
            format!("You played {} of This or That.", count(rounds, "round")),
            format!("どっち派？を{rounds}問遊びました。"),
        ),
        ("thisorthat", Some(rounds), _, _, _) if rounds > 0 => {
            let same = num("same_pick").unwrap_or(0);
            (
                match same {
                    0 => format!(
                        "You picked differently every time, in {} of This or That.",
                        count(rounds, "round")
                    ),
                    1 => format!(
                        "You picked the same once in {} of This or That.",
                        count(rounds, "round")
                    ),
                    _ => format!(
                        "You picked the same {same} times in {} of This or That.",
                        count(rounds, "round")
                    ),
                },
                format!("どっち派？で{rounds}問中{same}回、同じほうを選びました。"),
            )
        }
        ("story", _, _, _, _) if num("sentences").unwrap_or(0) > 0 => {
            let lines = num("sentences").unwrap_or(0);
            (
                format!(
                    "You wrote a story together, {} long.",
                    count(lines, "sentence")
                ),
                format!("二人で{lines}文の物語を書きました。"),
            )
        }
        _ => (
            format!("You spent time together on {}.", name("en")),
            format!("「{}」を一緒に楽しみました。", name("ja")),
        ),
    };
    Bi::new(
        format!(
            "{en} Next time: {}, and something new.",
            count(mins, "minute")
        ),
        format!("{ja}次回は{mins}分、別のことをしましょう。"),
    )
}

/// What the model is asked.
pub fn question(f: &Value) -> (String, String) {
    let system = "Two people just did a small activity together in Duet, and both said they want to meet again tomorrow. Write the short note both of them see.\n\
One sentence on what they did, one on next time.\n\
Use only the facts given. Every number you write must be one of the numbers in the facts. Do not invent what they said or felt. No names, links, times or dates.\n\
Warm, plain, friendship only, under 140 characters in each language.\n\
Reply with one JSON object and nothing else: {\"en\": \"English text\", \"ja\": \"natural Japanese with the same meaning\"}"
        .to_string();
    (system, f.to_string())
}

/// Every number in the facts, as text.
fn allowed_numbers(f: &Value, out: &mut Vec<String>) {
    match f {
        Value::Number(n) => out.push(n.to_string()),
        Value::Array(a) => a.iter().for_each(|v| allowed_numbers(v, out)),
        Value::Object(o) => o.values().for_each(|v| allowed_numbers(v, out)),
        _ => {}
    }
}

/// The numbers written in a line, with full-width digits read as plain ones.
fn numbers_in(text: &str) -> Vec<String> {
    let mut out = words_as_numbers(text);
    let mut cur = String::new();
    for c in text.chars() {
        let d = match c {
            '0'..='9' => Some(c),
            '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32),
            _ => None,
        };
        match d {
            Some(d) => cur.push(d),
            None if !cur.is_empty() => out.push(std::mem::take(&mut cur)),
            None => {}
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|n| n.trim_start_matches('0').to_string())
        .map(|n| if n.is_empty() { "0".into() } else { n })
        .collect()
}

/// Numbers written out: English words up to twenty (and "dozen", "hundred"), and kanji numerals
/// up to ninety-nine. A lone 一 is left alone: 一緒 and 一枚 are everyday Japanese, not counts.
fn words_as_numbers(text: &str) -> Vec<String> {
    const EN: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    let mut out = Vec::new();
    for word in text.to_lowercase().split(|c: char| !c.is_alphabetic()) {
        if let Some(n) = EN.iter().position(|w| *w == word) {
            out.push(n.to_string());
        } else if word == "dozen" {
            out.push("12".into());
        } else if word == "hundred" {
            out.push("100".into());
        }
    }
    let digit = |c: char| "〇一二三四五六七八九".chars().position(|d| d == c);
    let mut run: Vec<char> = Vec::new();
    let mut flush = |run: &mut Vec<char>| {
        let value = match run.as_slice() {
            [] | ['一'] => None,
            [d] if *d != '十' => digit(*d),
            ['十'] => Some(10),
            ['十', u] => digit(*u).map(|u| 10 + u),
            [t, '十'] => digit(*t).map(|t| t * 10),
            [t, '十', u] => digit(*t).zip(digit(*u)).map(|(t, u)| t * 10 + u),
            _ => Some(usize::MAX), // something longer: read as a number that is never allowed
        };
        if let Some(v) = value {
            out.push(v.to_string());
        }
        run.clear();
    };
    for c in text.chars() {
        if c == '十' || digit(c).is_some() {
            run.push(c);
        } else {
            flush(&mut run);
        }
    }
    flush(&mut run);
    out
}

/// The model's note, if it holds up: both languages, short, safe, and no number that is not in
/// the facts.
pub fn checked(f: &Value, reply: &Value) -> Option<Bi> {
    let mut allowed = Vec::new();
    allowed_numbers(f, &mut allowed);
    let part = |key: &str| {
        let raw = reply.get(key)?.as_str()?;
        if raw.trim().chars().count() > MAX_CHARS {
            return None;
        }
        let text = clean_text(&raw.replace('\n', " "), MAX_CHARS + 1)?;
        let ok = text.chars().count() <= MAX_CHARS
            && safe(&text)
            && numbers_in(&text).iter().all(|n| allowed.contains(n));
        ok.then_some(text)
    };
    Some(Bi::new(part("en")?, part("ja")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wl() -> Value {
        facts(
            "wavelength",
            &json!({"kind": "wavelength", "rounds": 8, "synced": 5}),
            880,
            45,
        )
    }

    #[test]
    fn the_rules_note_uses_the_record() {
        let note = rules(&wl());
        assert!(note.en.contains("5 times in 8 rounds"), "{}", note.en);
        assert!(note.en.contains("45 minutes"));
        assert!(note.ja.contains("8問中5回"));
        let sketch = rules(&facts("sketch", &json!({"strokes": 41}), 900, 45));
        assert!(sketch.en.contains("41 strokes"));
        let one = rules(&facts(
            "wavelength",
            &json!({"rounds": 1, "synced": 1}),
            60,
            45,
        ));
        assert!(one.en.contains("once in 1 round of"), "{}", one.en);
        let soon = rules(&facts("signal", &json!({"levels": 2}), 60, 1));
        assert!(soon.en.contains("Next time: 1 minute,"), "{}", soon.en);
        let missed = rules(&facts(
            "wavelength",
            &json!({"rounds": 3, "synced": 0}),
            60,
            45,
        ));
        assert!(missed.en.contains("played 3 rounds"), "{}", missed.en);
        let stroke = rules(&facts("sketch", &json!({"strokes": 1}), 60, 45));
        assert!(stroke.en.contains("1 stroke between"), "{}", stroke.en);
        let picks = rules(&facts(
            "thisorthat",
            &json!({"rounds": 8, "same": 3}),
            60,
            45,
        ));
        assert!(
            picks.en.contains("the same 3 times in 8 rounds"),
            "{}",
            picks.en
        );
        let tale = rules(&facts("story", &json!({"lines": 6}), 60, 45));
        assert!(
            tale.en.contains("6 sentences long") && tale.ja.contains("6文"),
            "{}",
            tale.en
        );
        let nothing = rules(&facts("signal", &Value::Null, 120, 45));
        assert!(nothing.en.contains("Signal"), "{}", nothing.en);
    }

    #[test]
    fn a_page_cannot_send_huge_numbers() {
        let f = facts("sketch", &json!({"strokes": 123_456_789}), 900, 45);
        assert_eq!(f["record"]["strokes"], 999);
    }

    #[test]
    fn a_model_note_with_only_known_numbers_is_taken() {
        let reply = json!({"en": "In sync 5 times out of 8! Tomorrow: 45 minutes of something new.", "ja": "８問中５回一致！明日は45分、新しいことを。"});
        let note = checked(&wl(), &reply).expect("all numbers are in the facts");
        assert!(note.ja.contains("５回"));
    }

    #[test]
    fn numbers_in_words_are_checked_too() {
        let made_up = json!({"en": "You laughed twelve times!", "ja": "十二回も笑いました。"});
        assert_eq!(checked(&wl(), &made_up), None);
        let kanji = json!({"en": "In sync 5 times out of 8.", "ja": "八問中五回、気持ちが一致。一緒にまた。"});
        assert!(
            checked(&wl(), &kanji).is_some(),
            "八 and 五 are in the record, and 一緒 is not a count"
        );
        let wrong = json!({"en": "In sync 5 times.", "ja": "九問中五回。"});
        assert_eq!(checked(&wl(), &wrong), None);
    }

    #[test]
    fn a_note_one_word_too_long_is_refused_not_cut() {
        let long = json!({"en": format!("{} tail", "a".repeat(160)), "ja": "はい"});
        assert_eq!(checked(&wl(), &long), None);
    }

    #[test]
    fn the_rules_note_says_only_what_is_known() {
        assert!(
            !rules(&facts("thisorthat", &json!({"rounds": 5}), 60, 45))
                .en
                .contains("differently every time")
        );
        let boast = rules(&facts(
            "wavelength",
            &json!({"rounds": 3, "synced": 999}),
            60,
            45,
        ));
        assert!(
            !boast.en.contains("999") && boast.en.contains("3 times in 3 rounds"),
            "{}",
            boast.en
        );
        assert!(
            rules(&facts("signal", &json!({"levels": 12}), 60, 45))
                .ja
                .contains("12本")
        );
    }

    #[test]
    fn a_model_note_with_an_invented_number_is_refused() {
        let made_up =
            json!({"en": "You laughed 12 times! See you at 7.", "ja": "12回笑いました。"});
        assert_eq!(checked(&wl(), &made_up), None);
        let one_language = json!({"en": "Lovely session. 45 minutes next time."});
        assert_eq!(checked(&wl(), &one_language), None);
        let link = json!({"en": "See www.example.com", "ja": "www.example.com を見て"});
        assert_eq!(checked(&wl(), &link), None);
    }
}

//! Each tool, as it is called.

use super::*;

impl Server {
    pub(super) fn tool(&mut self, name: &str, args: &Value) -> anyhow::Result<Value> {
        let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        let number = |k: &str| args.get(k).and_then(Value::as_u64).map(|n| n as usize);
        let account = text("account");
        let folder = text("folder").unwrap_or_else(|| "INBOX".to_string());
        match name {
            "list_accounts" => {
                let names = self.names(account.as_deref());
                if names.is_empty() && account.is_some() {
                    // Says what there is.
                    self.store(account.as_deref())?;
                }
                let version = env!("CARGO_PKG_VERSION");
                if args.get("names_only").and_then(Value::as_bool) == Some(true) {
                    let names: Vec<Value> = names.iter().map(|n| json!({ "name": n })).collect();
                    return Ok(json!({ "letterbox": version, "accounts": names }));
                }
                let mut all = Vec::new();
                for n in names {
                    let listed = self
                        .store(Some(&n))
                        .and_then(|s| folders_counted(s.as_mut()));
                    all.push(match listed {
                        Ok(folders) => json!({ "name": n, "folders": folders }),
                        Err(e) => {
                            json!({ "name": n, "error": said(&e) })
                        }
                    });
                }
                Ok(json!({ "letterbox": version, "accounts": all }))
            }
            "list_messages" => {
                let limit = number("limit").unwrap_or(20).clamp(1, LISTED_LIMIT);
                let offset = number("offset").unwrap_or(0);
                let page = self
                    .store(account.as_deref())?
                    .page(&folder, offset, limit)?;
                let shown = page.summaries.len();
                Ok(json!({
                    "account": self.called(account.as_deref()),
                    "folder": folder,
                    "total": page.total,
                    "offset": offset,
                    "shown": shown,
                    "messages": page.summaries.iter().map(summary).collect::<Vec<_>>(),
                    "more": offset + shown < page.total as usize,
                }))
            }
            "nearest_messages" => {
                let uid = number("uid").context("give the message's uid")? as u32;
                let limit = number("limit").unwrap_or(10).clamp(1, 50);
                let i = self.on_map(account.as_deref(), &folder, uid)?;
                let near: Vec<(crate::map::Dot, f32)> = {
                    let s = &self.mapped()?.snapshot;
                    s.model
                        .nearest(i, limit)
                        .into_iter()
                        .map(|(j, d)| (s.dots[j].clone(), d))
                        .collect()
                };
                let mut nearest = Vec::new();
                for (d, degrees) in near {
                    let place = d.places.first().cloned();
                    nearest.push(json!({
                        "account": self.name_of(&d.key.account),
                        "folder": place.as_ref().map(|p| p.folder.clone()),
                        "uid": place.map(|p| p.uid),
                        "from": mail::clean(&d.from),
                        "subject": mail::clean(&d.subject),
                        "date": d.date,
                        "labels": d.labels,
                        "degrees": (degrees * 10.0).round() / 10.0,
                    }));
                }
                Ok(json!({ "nearest": nearest }))
            }
            "suggest_label" => {
                let uid = number("uid").context("give the message's uid")? as u32;
                let i = self.on_map(account.as_deref(), &folder, uid)?;
                let s = &self.mapped()?.snapshot;
                let go = s.suggestion(i).map(str::to_string);
                let mut labels: Vec<(String, f32)> = s
                    .model
                    .label_vectors
                    .values()
                    .map(|(name, v)| (name.clone(), crate::model::degrees(&s.model.vectors[i], v)))
                    .collect();
                labels.sort_by(|a, b| {
                    let first = |n: &str| Some(n) != go.as_deref();
                    first(&a.0).cmp(&first(&b.0)).then(a.1.total_cmp(&b.1))
                });
                labels.truncate(5);
                Ok(json!({
                    "would_go_to": go,
                    "labels": labels
                        .into_iter()
                        .map(|(name, d)| json!({ "name": name, "degrees": (d * 10.0).round() / 10.0 }))
                        .collect::<Vec<_>>(),
                }))
            }
            "would_throw_away" => {
                let uid = number("uid").context("give the message's uid")? as u32;
                let i = self.on_map(account.as_deref(), &folder, uid)?;
                let m = self.mapped()?;
                let (odds, sure, why) = m
                    .junk(&m.snapshot.dots[i].key)
                    .context("not in the index")?;
                let f = &m.index.filter;
                Ok(json!({
                    "odds": odds,
                    "ready": f.ready(),
                    "would_throw_away": sure,
                    "sure_at": crate::filter::SURE,
                    "why": why,
                    "taught": { "thrown": f.thrown, "kept": f.kept },
                }))
            }
            "recent_messages" => Ok(self.recent_all(args, false)),
            "upcoming_events" => {
                let days = args
                    .get("days")
                    .and_then(Value::as_i64)
                    .unwrap_or(7)
                    .clamp(1, 60);
                let read = if self.demo {
                    Some(crate::agenda::Read {
                        coming: crate::agenda::demo(),
                        calendars: vec![crate::agenda::CalendarRead {
                            id: crate::agenda::USER.to_string(),
                            name: Some("Demo".to_string()),
                            fault: None,
                            coming: Vec::new(),
                        }],
                        ..Default::default()
                    })
                } else {
                    crate::agenda::read(&self.config, days)?
                };
                let Some(read) = read else {
                    return Ok(json!({
                        "events": [],
                        "said": "no calendar is set: `lbx calendar add` reads one, or use a calendar tool of your own",
                    }));
                };
                let events = crate::agenda::for_agent(&read.coming, &jiff::tz::TimeZone::system());
                Ok(json!({
                    "events": events,
                    "calendars": crate::agenda::names(&read.calendars),
                    "not_read": crate::agenda::faults(&read.calendars),
                    "left_out": read.unread.rules + read.unread.events,
                }))
            }
            "triage" => Ok(self.recent_all(args, true)),
            "trash_messages" | "file_messages" => {
                let uids = uids(args)?;
                let moving = self.moving(account.as_deref(), &folder, &uids)?;
                let thrown = name == "trash_messages";
                let n = moving.len();
                let action = if thrown {
                    crate::report::Action::Trash
                } else {
                    crate::report::Action::File
                };
                let folders = self.store(account.as_deref())?.folders()?;
                let of_drafts = folders
                    .iter()
                    .any(|f| f.path == folder && matches!(f.role, mail::Role::Drafts));
                let to = self.destination(
                    account.as_deref(),
                    &folders,
                    &folder,
                    action,
                    text("to").as_deref(),
                )?;
                let (what, yes) = if thrown {
                    let them = if n == 1 { "it" } else { "them" };
                    (
                        format!("Move {n} from {folder} to {to}? letterbox can put {them} back."),
                        "Move them to Trash",
                    )
                } else {
                    (format!("File {n} from {folder} under {to}?"), "File them")
                };
                let answer = match self.confirm(&listing(&what, &moving, of_drafts), yes)? {
                    Said::Yes => None,
                    Said::No => Some("the user said no: nothing was moved"),
                    Said::Unanswered => Some(
                        "the user did not answer (the question was dismissed, or no one was \
                         there to see it): nothing was moved",
                    ),
                };
                if let Some(answer) = answer {
                    return Ok(json!({ "moved": 0, "said": answer }));
                }
                let mut moved = Vec::new();
                let mut failed = Vec::new();
                let mut trashed = Vec::new();
                let at = jiff::Timestamp::now().as_second();
                let store = self.store(account.as_deref())?;
                let address = store.address().to_string();
                for s in &moving {
                    let done = if thrown {
                        store
                            .relocate(&folder, s.uid, &to, &s.message_id)
                            .map(|uid| {
                                trashed.push(crate::trash::Entry {
                                    account: address.clone(),
                                    message_id: s.message_id.clone(),
                                    folder: to.clone(),
                                    uid,
                                    origin: folder.clone(),
                                    at,
                                })
                            })
                    } else {
                        store
                            .file_from(&folder, s.uid, &to, &s.message_id, &folder)
                            .map(|_| ())
                    };
                    match done {
                        Ok(()) => moved.push(s.uid),
                        Err(e) => failed.push(json!({
                            "uid": s.uid,
                            "error": said(&e),
                        })),
                    }
                }
                // Moved, whether or not it is recorded to be put back: said apart.
                let unrecorded = (!self.demo && !trashed.is_empty())
                    .then(|| crate::trash::Ledger::load().record_many(trashed).err())
                    .flatten()
                    .map(|e| said(&e));
                let gone: Vec<&Summary> =
                    moving.iter().filter(|s| moved.contains(&s.uid)).collect();
                // A draft is the user's own, not what they would throw away
                // of what comes to them, wherever All Mail holds it too.
                let learnt = if of_drafts {
                    json!(0)
                } else {
                    self.advise(account.as_deref(), &folder, &gone, thrown)
                };
                Ok(json!({
                    "moved": moved.len(),
                    "not_recorded_to_put_back": unrecorded,
                    "to": to,
                    "uids": moved,
                    "failed": failed,
                    "learnt": learnt,
                }))
            }
            "search_messages" => {
                let query = text("query").context("give the words to search for")?;
                let limit = number("limit").unwrap_or(20).clamp(1, LISTED_LIMIT);
                let found = self
                    .store(account.as_deref())?
                    .search(&folder, &query, limit)?;
                Ok(json!({
                    "account": self.called(account.as_deref()),
                    "folder": folder,
                    "query": query,
                    "found": found.len(),
                    "messages": found.iter().map(summary).collect::<Vec<_>>(),
                }))
            }
            "read_message" => {
                let uid = number("uid").context("give the message's uid")? as u32;
                let from = number("from_char").unwrap_or(0);
                let chars = number("max_chars").unwrap_or(CHARS).clamp(1, CHARS_LIMIT);
                let raw = self.store(account.as_deref())?.raw(&folder, uid)?;
                let m = mail::message(uid, raw)?;
                let total = m.text.chars().count();
                let body: String = m.text.chars().skip(from).take(chars).collect();
                let to = from + body.chars().count();
                Ok(json!({
                    "account": self.called(account.as_deref()),
                    "uid": m.uid,
                    "folder": folder,
                    "subject": m.subject,
                    "from": m.from,
                    "to": m.to,
                    "cc": m.cc,
                    "reply_to": m.reply_to,
                    "date": m.date.map(date),
                    "message_id": m.message_id,
                    "attachments": m.attachments,
                    "links": m.links,
                    "text": body,
                    "text_chars": total,
                    "more": (to < total).then(|| format!("from_char {to} for more")),
                }))
            }
            "uncertain_messages" => {
                let limit = number("limit").unwrap_or(5).clamp(1, 20);
                self.uncertain(limit)
            }
            "ask_about_messages" => {
                let uids = uids(args)?;
                let moving = self.moving(account.as_deref(), &folder, &uids)?;
                self.ask_about(account.as_deref(), &folder, &moving)
            }
            "draft_reply" | "draft_message" => self.draft(name, args),
            "report_triage" => {
                let report = self.report(args)?;
                let proposed = report.proposals.len();
                let shown = match self.mapped()?.advise.clone() {
                    Some((dir, recipient)) => {
                        crate::index::journal::append(
                            &dir,
                            crate::report::JOURNAL,
                            &recipient,
                            vec![report],
                        )?;
                        "letterbox shows it as it next can: the user chooses there what is done"
                    }
                    None => {
                        self.reports.push(report);
                        "kept, as the demo keeps it"
                    }
                };
                Ok(json!({ "proposed": proposed, "said": shown }))
            }
            "open_letterbox" => {
                if self.demo {
                    return Ok(
                        json!({ "opened": false, "said": "the demo opens nothing: lbx --demo does" }),
                    );
                }
                if crate::index::is_open(&crate::index::key::dir()) {
                    return Ok(json!({
                        "opened": false,
                        "said": "letterbox is open already: what was reported shows there",
                    }));
                }
                let command = open_command(
                    config::opener(&self.config)?.as_deref(),
                    &std::env::current_exe()?,
                )
                .context(
                    "say how to open letterbox in a terminal, as open in [agent] in accounts.toml",
                )?;
                config::shell(&command)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .context("cannot open letterbox")?;
                Ok(json!({
                    "opened": true,
                    "said": "letterbox opens in a window of its own: what was reported pops up there",
                }))
            }
            "mark_message" => {
                let uid = number("uid").context("give the message's uid")? as u32;
                let seen = args.get("seen").and_then(Value::as_bool);
                let flagged = args.get("flagged").and_then(Value::as_bool);
                if seen.is_none() && flagged.is_none() {
                    bail!("say seen, flagged, or both");
                }
                let store = self.store(account.as_deref())?;
                for (flag, on) in [(Flag::Seen, seen), (Flag::Flagged, flagged)] {
                    if let Some(on) = on {
                        store.set(&folder, uid, flag, on)?;
                    }
                }
                Ok(json!({ "uid": uid, "folder": folder, "seen": seen, "flagged": flagged }))
            }
            _ => bail!("no tool {name}"),
        }
    }
}

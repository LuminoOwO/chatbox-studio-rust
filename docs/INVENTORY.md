# Chatbox Studio - complete feature inventory for the Rust rewrite

This is the checklist the Rust version has to match. It lists everything the Python version
(**[chatbox-studio](https://github.com/LuminoOwO/chatbox-studio) 1.31**, ~15,800 lines in 33
modules) does: features, settings, file formats, outside interfaces and behaviour details.
The goal is **the same app, rewritten** - no new features, nothing dropped, and existing
settings files keep working.

Each item is a checkbox. It's ticked when the Rust version does the same thing.
The Python module it comes from is given in brackets, e.g. *(engine.py)*.

Contents:

1. [Speech pipeline](#1-speech-pipeline)
2. [Translation](#2-translation)
3. [Routes, chatbox format and splitting](#3-routes-chatbox-format-and-splitting)
4. [Filters](#4-filters)
5. [VRChat connection (OSC, OSCQuery)](#5-vrchat-connection-osc-oscquery)
6. [Chatbox composer (blocks)](#6-chatbox-composer-blocks)
7. [Chatbox editor (GUI)](#7-chatbox-editor-gui)
8. [Live values: PC, VR, devices, audio](#8-live-values-pc-vr-devices-audio)
9. [Music, program audio, active app, lyrics](#9-music-program-audio-active-app-lyrics)
10. [Earshot (experimental)](#10-earshot-experimental)
11. [Graphics cards and engines](#11-graphics-cards-and-engines)
12. [Main window](#12-main-window)
13. [Settings window](#13-settings-window)
14. [Help: quick start, tour, checklist](#14-help-quick-start-tour-checklist)
15. [Themes, text size, fonts, accessibility](#15-themes-text-size-fonts-accessibility)
16. [Setup (installer window)](#16-setup-installer-window)
17. [Installing, packages, updates](#17-installing-packages-updates)
18. [Files, paths and formats](#18-files-paths-and-formats)
19. [Robustness: logs, crashes, single instance](#19-robustness-logs-crashes-single-instance)
20. [Platform differences](#20-platform-differences)
21. [Languages](#21-languages)
22. [Build, release, CI](#22-build-release-ci)
23. [Rust architecture](#23-rust-architecture)
24. [Order of work](#24-order-of-work)
25. [Open questions and risks](#25-open-questions-and-risks)

---

## 1. Speech pipeline

*(engine.py: `StreamingVAD`, `Segmenter`, `Transcriber`, `Engine`)*

### Microphone
- [ ] List input devices; on Windows prefer the default host API (WASAPI) entries; resolve a
      saved device by name when its index changed. *(list_microphones, resolve_microphone)*
- [ ] Linux: `pipewire` / `default` devices offered.
- [ ] Capture 16 kHz mono float32 (resample with soxr-quality resampling when the device
      can't do 16 kHz).
- [ ] Restart audio on device change without restarting the app.
- [ ] Level meter value: RMS × 8, clipped to 1; "voice" flag when VAD prob ≥ threshold.

### Voice detection (Silero VAD v6, ONNX, CPU)
- [ ] 512-sample frames (32 ms at 16 kHz) with a 64-sample context and the h/c state
      (128 each); one thread, onnxruntime CPU.
- [ ] `vad_threshold` (default 0.5).
- [ ] Segmenter: 320 ms pre-roll (10 frames), keep 160 ms silence tail (5 frames), sentence
      ends after `end_silence_ms` (500), dropped when speech < `min_speech_ms` (250),
      forced cut at `max_utterance_s` (10 s) that keeps listening (sentence continues).
- [ ] Reset VAD state on pause / mute.

### Whisper (faster-whisper models on CTranslate2)
- [ ] Models: small, medium, large-v2, large-v3 (+ `.en` variants refused for non-English
      sources with the message "English-only models (*.en) only understand English.").
- [ ] **One encoder pass per sentence**; language detection and every decode reuse it.
- [ ] Language detection only among the route source languages; probability renormalised
      over those; single source → no detection.
- [ ] Per source language a plan: want English (Whisper translate task) and/or original
      (transcribe task); `en` source → original only.
- [ ] Decoding: prompt = optional previous-text (`sot_prev` + last 150 tokens of context)
      + SOT sequence + `no_timestamps`; `beam_size` (1 = greedy default),
      `max_length = min(440, 200 + len(prompt))`, scores + no-speech prob, suppress blank,
      faster-whisper's suppressed tokens.
- [ ] avg_logprob = score × len / (len + 1).
- [ ] Drop rules: no alphanumerics → "empty"; nsp > 0.6 and alp < -1.0 → "no speech";
      hallucination regex or known phrase with nsp > 0.2 → "hallucination".
      Hallucination list and regex exactly as in engine.py (Amara, "thanks for watching",
      subtitle credits incl. "Untertitel von/im Auftrag", ZDF, "copyright ©").
- [ ] Warm-up decode at load (1 s silence) so the first sentence isn't slow.
- [ ] compute type "auto": int8 on Pascal (cc < 7), int8_float16 on newer NVIDIA, AMD order
      int8_float16 / float16 / int8; CPU int8; user override.
- [ ] Result: text, language, language probability, duration, latency ms, original,
      english, dropped + reason.
- [ ] Earshot room mode `run_room` (any language, lprob ≥ 0.35, stricter noise rules,
      optional English).

### Engine loop
- [ ] Separate VAD thread and ASR thread; queue between them.
- [ ] States emitted to the UI: loading / ready / listening / speaking / processing / paused
      / error, model name, device.
- [ ] Pause / resume listening (Ctrl+P); "Pause when muted in VRChat" (follows MuteSelf).
- [ ] Typing indicator `/chatbox/typing` while speaking (setting `typing_indicator`).
- [ ] Optional notify sound on send (`notify_sound`).
- [ ] Reload model when model / device / compute type changes ("Apply" button state).
- [ ] Typed messages (`send_manual`): filters optional (`filter_typed`), translation
      optional (`translate_typed`, language = first route's source).
- [ ] Transcript entries with latency (ms), language, outputs, orange notes for
      untranslated sentences and why.

## 2. Translation

*(translator.py, engine.py `_translate`, smart.py)*

- [ ] NLLB-200 distilled 600M (default) and 1.3B, CTranslate2 int8 conversions, downloaded
      into the models folder (`model_path`, `is_installed`, download with progress).
- [ ] Whisper translates to English by itself; NLLB for every other target, and for English
      too when `nllb_for_english`.
- [ ] Translator device (`translator_device`: same / nvidia:N / amd:N / cpu; Vulkan → CPU).
- [ ] Translator states: not installed / downloading (percent) / loading / ready / error;
      shown at the top ("Translator ready").
- [ ] Offer to install the translator when a route needs it (`maybe_offer_install`).
- [ ] Warm-up translation at load.
- [ ] Linux helper script `setup_translator.sh` (build the NLLB files from Meta's model).
- [ ] Several translations joined by `join_with`: newline | bar | slash.
- [ ] Smart translation (Earshot, Qwen2.5 3B/7B CTranslate2 int8 from
      `jncraton/Qwen2.5-*-Instruct-ct2-int8`, custom folder): ChatML prompt with the recent
      conversation, output cleaning, falls back to NLLB on garbage (see §10).

## 3. Routes, chatbox format and splitting

*(engine.py `active_routes`, `format_chatbox`, `split_for_chatbox`; routes_widget.py)*

- [x] Routes `{src, dst, enabled}`; default `[{en → en}]`; several active at once; source
      list = Whisper's allowed languages.
- [x] `dst = "room"`: Earshot's room language (falls back to `room_fallback`).
- [ ] Routes panel: language combo boxes (47 languages, English first then alphabetical),
      enable checkbox, ⇄ reverse button, remove, **+ Add route** (new row en → en),
      Presets menu: "Only English, no translation", "Add the other direction of every
      route", "Translate everything I say into English".
- [x] Chatbox template with `{text} {original} {src} {dst}` (`_SafeDict`: unknown keys stay
      literally); templates with `{original}` fall back to `{text}` when nothing was
      translated.
- [x] Split long text into 144-character messages (VRChat limit) at word boundaries;
      **UTF-16 length counting** (emoji = 2).
- [ ] Save transcript to a text file.

## 4. Filters

*(filters.py, filters_dialog.py)*

- [x] Filler words removal (`remove_fillers`, list incl. "um", "uh", "äh", ...).
- [x] Replace rules: find → replace, whole word, regex, case-insensitive matching that keeps
      the original capitalisation (`_keep_case`), empty replacement deletes.
- [x] Block rules: sentence containing a phrase is not sent (logged).
- [x] `min_words`: ignore shorter sentences.
- [ ] `filters_enabled`, `filter_typed`.
- [ ] Filters dialog: two rule tables (add / remove rows), options, "Try it" box showing
      exactly what would be sent (`FilterOutcome`).
- [ ] Filter summary line in settings.

## 5. VRChat connection (OSC, OSCQuery)

*(engine.py `OscBridge`, oscquery.py)*

- [x] Send `/chatbox/input [text, True, notify]` and `/chatbox/typing [bool]` over UDP to
      `osc_host:osc_port` (127.0.0.1:9000).
- [x] Listen on `osc_listen_port` (9001) for `/avatar/parameters/MuteSelf`,
      `/avatar/parameters/AFK` and all `/avatar/parameters/*` (VRChat values: vrc_mode,
      vrc_tracking, vrc_height, vrc_scale, vrc_seated, vrc_earmuffs).
- [x] Port busy (VOR, face tracking): listen on a free port and advertise it with
      **OSCQuery** (`oscquery`): mDNS services `_oscjson._tcp` + `_osc._udp` and a small HTTP
      server answering the host info and the `/avatar` tree, so VRChat sends to us.
- [x] **VRChatFinder**: browse mDNS for VRChat's own OSCQuery service and send the chatbox
      to VRChat's real port (fixes chatbox through OSC routers); go back to 9000 when VRChat
      closes ("VRChat closed - chatbox goes to port 9000 again.").
- [x] Retry loop when the port becomes free again ("mute sync works now").
- [ ] Listen state shown in settings and the checklist ("Mute sync: port 9001 busy").
- [ ] Rate limit: at most every `min_send_interval_s` (1.5 s) for speech; chatbox composer
      `min_interval` 1.2 s and `keepalive` 20 s.

## 6. Chatbox composer (blocks)

*(chatbox.py, blocks.py)*

### Blocks and config
- [x] Block types: `speech` (one), `value` ({key}), `text` (own text with values; shows only
      when all its values are non-empty; `[ ... ]` parts vanish when a value inside is
      empty), `rotating` (messages that take turns), `newline`.
- [x] Block fields: id (8 hex), type, `prio` number (speech 1, others 2; 0 = never removed),
      visible, enabled, icon, per-type options (`OPT_DEFAULTS`: timezone, style 24h/12h,
      countdown target + label, max_title), display options (`DISPLAY_DEFAULTS`: show_mode,
      win_chars, win_step, win_seconds, win_lines), speech options (`SPEECH_DEFAULTS`:
      width 144, mode pages, page_seconds, scroll_words, scroll_seconds, hold_seconds, flow
      live, limit), gpu_index for graphics values, media filters.
- [x] Config: elements, separator (newline / custom), custom_separator, speech_alone,
      min_interval 1.2, keepalive 20, hide_extras.
- [x] **Migration** of older configs (elements with priority words high/normal/low/keep →
      numbers via `PRIO_NUM`; renamed keys `_RENAME`), `normalize`, `default_blocks`.
- [x] Spacing rules: two value blocks get a space between them; text blocks joined exactly;
      a line where every block is empty disappears.
- [x] Priority: when full, the highest number makes room first (all blocks with that number
      together); while speaking only blocks with a higher number than speech make room.
- [ ] Privacy groups (speech, text, time, music, status, pc, vr, vrchat) + **Hide extras**;
      clear chatbox (Ctrl+L).

### Values *(OWN_KEYS + sysinfo PLACEHOLDERS, 117 + own)*
- [x] Time & date: time, date, weekday, tz (any time zone, 12/24 h; tzdata on Windows).
- [x] Countdown: countdown_left, countdown_label, countdown_date, countdown_done.
- [ ] Music: title, artist, album, player, site, position, duration, bar (━━●──), status,
      audio_app, lyric_line, lyric_next.
- [ ] Status: afk, afk_time, playtime, app_time, active_app, window_title, room_lang.
- [ ] All PC / VR / device values of §8.

### Display modes *(chatbox.py frame functions)*
- [x] Values / text: full, Ticker, Marquee, Bounce, Lines, Pages, Typewriter (window of
      `win_chars`, `win_step`, `win_seconds`, `win_lines`).
- [x] Speech: Pages, Scroll (`scroll_words`/`scroll_seconds`), Marquee, Typewriter;
      `hold_seconds`; flow live; `limit`.
- [x] `truncate16` / `ulen` UTF-16 aware truncation; `wrap_words`; progress bar;
      `fmt_duration`, `fmt_clock`.
- [ ] Stats watcher only collects the values in use (`want`, `hold`), screen info.
- [ ] VRChat clock (play time), AFK time.

## 7. Chatbox editor (GUI)

*(chatbox_tab.py)*

- [ ] **Block list** (palette): categories (Speech, Text & layout, Time & date, Countdown,
      Music, Status, System, CPU, Graphics, Memory, Storage, Network, Display, Power,
      VR headset, WiVRn*, Controllers & trackers, Audio, Devices*, VRChat), search box, live
      current value per entry, text starters (Text, separators, "CPU: {cpu}%", "🎵 {title}[
      — {artist}]", "🕒 {time}", "💤 AFK for {afk_time}"). *Linux only.
- [ ] Drag from the list into the chatbox; Enter / double-click adds at the end;
      ↵ New line.
- [ ] **VRChat-accurate preview**: Noto Sans, black-and-white emoji, same text width and
      margins, max 9 lines (measured by vrc-live-caption); edit view (empty blocks greyed,
      line breaks visible) vs. exact view; character counter "n / 144" (UTF-16).
- [ ] Click a block to edit; drag to move; Alt+← / Alt+→; drop on the list / Delete /
      Remove to delete; drop a value onto a text block to embed it; **Undo** (Ctrl+Z).
- [ ] Block editor per type: icon, priority, speech options with live sample ("Characters
      shown at once"), value options (time zone, style, countdown, music title length and
      filters, lyrics source, GPU number), text editor with token highlighting and
      insert-value menu, rotating messages, display mode options.
- [ ] Privacy bar (QuickBar) with a button per group and **🙈 Hide extras**; Clear chatbox.
- [ ] **Music filters dialog**: mode "Show everything, except" / "Only show", rules
      (Website: …, Program: …, Title contains …), detected-right-now list with Hide / Allow
      buttons, "never shown" defaults (VRChat, Discord, SteamVR, …), fallback threshold.
- [ ] Preview status line ("sending off", what was sent).

## 8. Live values: PC, VR, devices, audio

*(sysinfo.py, ~1,350 lines)*

### Static (read once in the background)
- [ ] System: os, os_name, os_version, os_icon, kernel, desktop, session, pc_model, board,
      hostname, uptime, bios, arch, processes.
- [ ] CPU: cpu_name, cpu_short (`clean_cpu`), cores, threads, cpu_max, cpu_vendor, cpu_cache.
- [ ] Graphics: gpu_name, gpu_short (`clean_gpu`), gpus, gpu_driver, vram_total.
- [ ] Memory: ram_total, ram_speed (Windows), ram_type, ram_sticks, ram_maker.
- [ ] Storage: disk_model, disk_fs, disk_total, disks.
- [ ] Display: screen, refresh, screens, screen_count.

### Live (every couple of seconds, only what's used)
- [ ] CPU usage, frequency, temperature (Linux hwmon / k10temp / coretemp), fan.
- [ ] GPU usage, temperature, power, fan, clocks, power limit, PCIe, VRAM used
      (NVIDIA nvidia-smi; AMD sysfs `_amd_live_linux`).
- [ ] RAM used / %, swap; disk free / %, read / write rates, disk temp; network down / up
      rates (`fmt_rate`), type, link speed, Wi-Fi signal; battery %, state.

### VR
- [ ] SteamVR via OpenVR **as a background app** (never starts SteamVR): hmd, hmd_model,
      hmd_maker, hmd_refresh, hmd_battery, hmd_charging, ipd, vr_runtime, vr_link,
      vr_tracking, vr_connected, vr_session, hmd_serial; controllers (ctrl_model, batteries
      left/right), trackers, tracker battery/model, base stations, fbt.
- [ ] Linux: **WiVRn over D-Bus** (rates, bitrate, codec, features, hand/eye/face tracking,
      mic, speakers, batteries) and **Monado via libmonado** (devices, batteries, hmd_device);
      active OpenXR runtime from the manifests.
- [ ] Virtual Desktop / ALVR through SteamVR.

### Audio and devices
- [ ] audio_out, audio_in, volume, muted (Linux PulseAudio/PipeWire, Windows Core Audio).
- [ ] Linux **Devices**: wireless batteries from UPower, BlueZ, kernel sysfs batteries and
      `headsetcontrol`: devices, headphones / mouse / keyboard / gamepad / phone battery,
      device_count.
- [ ] Windows: Linux-only groups (WiVRn, Devices) and `hmd_device` not offered.

## 9. Music, program audio, active app, lyrics

*(media.py, lyrics.py)*

- [ ] Linux: MPRIS over D-Bus (title, artist, album, url, length, position, status, player,
      local file path); PipeWire / PulseAudio stream peak levels per program; X11 window
      titles (python-xlib) for the website and active app.
- [ ] Windows: System Media Transport Controls (WinRT), Core Audio session peak meters
      (pycaw), window titles (Win32).
- [ ] Choose the item to show (`choose`): playing over paused, filters (`rule_matches`,
      `is_allowed`), fallback to a program making sound (`fallback_threshold` 0.02) with its
      friendly name ("🔊 Soundpad"), never-shown defaults, show_paused.
- [ ] Website detection from URL or browser window title (`site_from_url`,
      `site_from_title`, `attach_sites`).
- [ ] Lyrics: LRC parsing, sources in order local `.lrc` next to the file, a chosen folder
      ("Artist - Title.lrc"), **LRCLIB** only when "Look up lyrics online" (sends title,
      artist, album, length); cache files; title cleaning; current and next line in time.

## 10. Earshot (experimental)

*(earshot.py, earshot_tab.py)*

- [ ] Settings (`earshot` dict, `DEFAULTS`): enabled, source (vrchat / all / device),
      device, whisper_context (off), echo_guard (on), show_heard, show_in, context_minutes 3,
      room_fallback, smart (off), smart_model, smart_device, listen_device, listen_model,
      smart_path, smart_english, vad_threshold; one-time migration `ctx_off_127`.
- [ ] Game audio: Linux PipeWire `pw-record` on VRChat's node (`find_vrchat_node`) or the
      monitor (`parec`), any input device; Windows loopback (soundcard) or a device.
- [ ] Own VAD + segmenter on game audio; transcription only while you're quiet, unless an
      own listening model on another device is loaded (states shared / loading / ready /
      error).
- [ ] Room context: lines with time, who (you / them), language, text; room language
      (majority of recent lines); Whisper prompt from your last sentences (max 1.5 min,
      160 chars) when enabled.
- [ ] Echo guard: drop your sentence when it matches what the game just played
      (`is_echo`, normalised text).
- [ ] Earshot tab: on/off, source, device, options, listening device + model, smart
      translation section (model, folder, install with progress, device), heard lines
      translated into `show_in`, clear, save.

## 11. Graphics cards and engines

*(gpus.py, devices.py, workers.py, model_worker.py, whispercpp.py, vram.py, gpu_panel.py)*

- [ ] Device strings: `cpu`, `nvidia:N` (nvidia-smi order), `amd:N` (ROCm), `vulkan:N`
      (Vulkan loader order), `same`; legacy `cuda` → same; `speech_spec` from old
      `gpu_index`.
- [ ] NVIDIA detection: nvidia-smi name, memory, compute capability.
- [ ] AMD detection: Linux sysfs (vendor 0x1002, VRAM, PCI order, device-id table `AMD_IDS`,
      gfx arch from IP discovery, APU GTT memory); Windows registry adapter memory.
- [ ] ROCm support table `ROCM_ARCHS`, Linux overrides `ROCM_OVERRIDE`
      (`HSA_OVERRIDE_GFX_VERSION`), APU table.
- [ ] Vulkan enumeration (in a helper process): name, type, vendor id, device id, heaps;
      skip CPU / virtual devices; integrated → shared memory estimate.
- [ ] Per-model device: speech, translator, smart AI, Earshot listening; translator / smart
      AI never on Vulkan (→ CPU with a log line).
- [ ] AMD models in a helper process with the ROCm build (length-prefixed messages over
      stdin/stdout, proxy objects, stop on exit).
- [ ] Vulkan speech via `whisper-server` (whisper.cpp 1.9.5, release asset of the Python
      repo, `GGML_VK_VISIBLE_DEVICES`, verbose_json, language probabilities, re-decode when
      the detected language isn't allowed, translate requests).
- [ ] **Live graphics panel**: per card in use (or all with the option): memory bar
      (Chatbox / others / free), temperature, 3-minute graph of load and memory, Chatbox's
      load share on Windows, tooltip with programs; pause while minimised.
- [ ] Memory measuring (`vram.py`): nvidia-smi totals + process table; AMD Linux sysfs +
      `/proc/*/fdinfo`; Windows PDH counters (GPU Adapter Memory, GPU Process Memory, GPU
      Engine utilization) matched to cards via DXGI LUIDs; "ours" = Chatbox processes;
      VRChat detection.

## 12. Main window

*(app.py)*

- [ ] Header: title, state pill (Loading / Ready / Listening / …), translator status,
      ❔ Help menu (F1), ⚙ Settings (Ctrl+,).
- [ ] Left panel ("quick panel", scrollable): Getting started checklist (hideable),
      microphone combo, level meter with threshold line, voice threshold, pause length
      ("Send after pause"), max sentence length, *Send to chatbox*, *Pause when muted*,
      Graphics cards section, "More settings…".
- [ ] Tabs: 🗣 Speech & translation (routes panel, transcript, typed message box with Send,
      Clear, Save…), 💬 Chatbox (§7), 🎧 Earshot (§10); Ctrl+1/2/3.
- [ ] Collapsible ▸ Log under the tabs; last message always visible; copy log.
- [ ] Keyboard: Ctrl+P pause, Ctrl+H hide extras, Ctrl+L clear chatbox, Ctrl+1/2/3,
      Ctrl+,, F1, Esc closes dialogs; Alt+←/→ in the editor; Ctrl+Z undo.
- [ ] Window state; close stops engine, helper processes and Vulkan servers.

## 13. Settings window

*(ui_help.py `SettingsDialog`, app.py `_build`)*

- [ ] One page per topic with search box: **General** (theme, text size, tips, show GPU
      usage, also unused cards, quick start again), **Speech recognition** (model, device,
      compute type, decoding Fastest / Most accurate, VAD values, Apply/Reload),
      **Translation** (translator model, device, use for English, translate typed,
      several translations joined by, chatbox format), **VRChat & OSC** (send to chatbox,
      host, port, listen port, OSCQuery, typing indicator, notify sound, mute sync state),
      **Filters** (summary + Edit filters…), **Parts & about** (open setup, version, data
      folders).
- [ ] Search filters controls by text across pages.

## 14. Help: quick start, tour, checklist

*(ui_help.py)*

- [ ] **Quick start** (first start when not `extra.onboarded`, 5 pages): Welcome, Your
      microphone (level test), Your languages ("I speak ▾ → chatbox shows ▾", "Also the other
      way round"), Connect VRChat (OSC on), You're ready; Skip / Back / Next.
- [ ] **Tour**: overlay pointing at each part of the window with text, Next / Back / Esc.
- [ ] **Getting started checklist**: model loaded, microphone heard a voice, VRChat OSC
      connected, translator ready (when needed) - each with a fix button.
- [ ] Help menu: Quick start, Tour, README (local file, else GitHub), copy log, about.

## 15. Themes, text size, fonts, accessibility

*(theme.py, chatbox_tab.py `load_fonts`)*

- [ ] Themes: Dark, Light, High contrast, Catppuccin Mocha / Macchiato / Frappé / Latte,
      Nord, Dracula, Gruvbox Dark, Tokyo Night, Rosé Pine - palette tokens exactly as in
      theme.py.
- [ ] Text size Small → Largest (`ui_scale`).
- [ ] Noto Sans + Noto Emoji (monochrome) for the chatbox preview.
- [ ] Everything reachable with Tab; tooltips; tips toggle.

## 16. Setup (installer window)

*(setup_gui.py, components.py, setup_tasks.py, httpzip.py)*

- [ ] Modes: first install, `--update`, `--modify`, `--first`, `--defaults` (no window).
- [ ] Simple page: PC line; **Keep what I have** (when installed), **Standard
      (recommended)**: speech + NLLB 600M, **Speech only**, **Custom…**; download size and
      what each installs.
- [ ] Custom page: speech model radios (sizes per engine), runs on (per device with engine
      name), translator on/off + model + device, graphics support (CUDA / ROCm / Vulkan,
      auto-ticked), extras (media, VR, core), Earshot (listening device + model, smart AI
      model + device).
- [ ] Footer: memory meter per card with **live use of other programs** (VRChat etc., hover
      list), fit check per card (ok / tight / no, with live free memory), disk meter, one-line
      summary in the simple view.
- [ ] Plan steps (download / install / remove), progress with percent, log, Stop, Fresh
      install, Uninstall completely (keep settings / models choice), Finish starts the app.
- [ ] Hardware detection, recommendation (`recommend`, `preset`, `default_gpu`,
      integrated / weak iGPU rules), `apply_settings` writes devices and models to the
      settings, `components.json` state.
- [ ] AMD on Linux: ROCm 7.2 system libraries check, *Install ROCm* (terminal with
      `install_rocm.sh`), the ROCm CTranslate2 wheel fetched with HTTP range requests.
- [ ] Vulkan engine download from the release, models from `ggerganov/whisper.cpp`.
- [ ] Exit codes: 3 Smart App Control blocked, 4 needs ROCm, 5 engine failed.

**In Rust most of this gets simpler:** there is no Python environment and no pip. The setup
becomes: choose parts → download models and GPU runtime libraries → done. See §23.

## 17. Installing, packages, updates

- [ ] Windows: NSIS setup .exe (per user, no admin) + ZIP; Start-menu and desktop
      shortcuts, Setup shortcut, uninstall entry; Smart App Control note.
- [ ] Linux: .deb, .rpm, Arch package (+ PKGBUILD), ZIP / tarball with `install.sh`;
      /opt install, menu entries (app + setup), icon, commands `chatbox-studio` and
      `chatbox-studio-setup`; take over older ZIP installs.
- [ ] Updates keep settings, models, routes, filters.

## 18. Files, paths and formats

*(paths.py)* - the Rust version reads and writes **the same files**:

| | Linux | Windows |
|---|---|---|
| Settings | `~/.config/chatbox-studio/settings.json` | `%APPDATA%\Chatbox Studio\settings.json` |
| Data (models, logs, state) | `~/.local/share/chatbox-studio` | `%LOCALAPPDATA%\Chatbox Studio` |
| Whisper models (CT2) | `~/.cache/huggingface` | `%USERPROFILE%\.cache\huggingface` |

- [x] `settings.json`: all fields of `Settings` (§1–5) with the same names and defaults,
      plus `chatbox`, `earshot`, `extra` (`onboarded`, `hide_gpu_monitor`,
      `gpu_monitor_all`, …); unknown keys kept.
- [ ] `components.json` (installed parts and devices).
- [ ] Models folder: NLLB, smart models, `ggml/` models, engines (`engines/rocm`,
      `engines/whispercpp`).
- [ ] `crash.log`, `starts.log` (who started / raised the app), lyrics cache.
- [x] Migration from the old app name folders (`migrate_from_old_name`).

## 19. Robustness: logs, crashes, single instance

- [ ] Crash handler writing `crash.log` with traceback.
- [ ] Single instance: a second start brings the window to the front (local socket with
      waits for a busy first instance); `starts.log` with parent process chain on Linux and
      the peer process of a raise.
- [ ] All optional sources fail quietly (empty values), logged once.

## 20. Platform differences

- [ ] Linux and Windows for everything; X11 and Wayland (website detection X11 only).
- [ ] Linux-only: WiVRn, Monado, Devices, PipeWire VRChat-only capture, ROCm system libs.
- [ ] Windows-only: SMTC, pycaw/Core Audio, PDH/DXGI memory, Smart App Control handling,
      registry GPU memory.

## 21. Languages

- [x] 47 languages with Whisper code, display name and NLLB (FLORES-200) code exactly as in
      languages.py; English first, then alphabetical; **no other language preferred**.

## 22. Build, release, CI

- [ ] Build every package from one command; release on version change in CI (same naming as
      now: `Chatbox-Studio-<v>-Windows-Setup.exe`, `chatbox-studio_<v>_amd64.deb`, …).
- [ ] Clanker Index badge workflow.

---

## 23. Rust architecture

### Crates (one Cargo workspace)

| Crate | Does | Main dependencies |
|---|---|---|
| `cbs-core` | settings + file formats, paths, languages, routes, chatbox format/splitting, filters, blocks + composer, display modes | serde, serde_json, regex, chrono + chrono-tz, unicode-segmentation |
| `cbs-audio` | microphone, resampling, Silero VAD, segmenter, game audio | cpal, rubato, ort (onnxruntime) |
| `cbs-asr` | Whisper engine (CTranslate2), whisper.cpp server client, NLLB, Qwen | ct2rs (CTranslate2), tokenizers, hf-hub |
| `cbs-osc` | OSC send / listen, OSCQuery (mDNS + HTTP), VRChat finder | rosc, mdns-sd, tiny_http |
| `cbs-sys` | PC values, GPUs, VRAM / load measuring, VR (OpenVR, WiVRn, Monado), devices, audio info | sysinfo, nvml-wrapper, openvr_sys / dlopen, zbus, windows-rs |
| `cbs-media` | now playing, program audio, active app, lyrics | zbus (MPRIS), libpulse-binding, x11rb, windows-rs (SMTC, Core Audio), ureq |
| `chatbox-studio` | the app (GUI) | eframe / egui |
| `chatbox-studio-setup` | the setup window | eframe / egui |

### Decisions to confirm

1. **GUI toolkit - egui (eframe), recommended.** One native binary, GPU-drawn, low memory,
   no system web view needed (Tauri would need WebKitGTK on Linux, which alone uses
   100–200 MB and differs per distribution). Drag and drop, custom drawing (the chatbox
   preview, graphs) and theming are straightforward. Downside: it can't share UI code with
   vrc-friend-map (Tauri). **Alternative: Tauri 2**, same stack as vrc-friend-map, web
   technology for the UI.
2. **Speech engine - CTranslate2 through `ct2rs`**, the same engine and model files as
   today, so recognition quality and speed stay identical. whisper.cpp stays for Vulkan.
   CUDA and ROCm builds of CTranslate2 are built in CI as shared libraries that the app
   loads at runtime - no Python, no pip.
3. **No Python environment at all.** The setup only downloads models and the GPU runtime
   libraries (cuBLAS/cuDNN from NVIDIA's packages, ROCm libraries, whisper.cpp).

### What stays the same on purpose

Settings file names and keys, default values, models and their download sources, OSC
behaviour, the chatbox rules, every text in the UI where possible, the package names.

## 24. Order of work

Each step builds, has tests, and is committed before the next:

1. ✅ **Workspace + `cbs-core`**: settings (read the Python app's settings.json unchanged),
   paths, languages, routes, chatbox format, 144-char UTF-16 splitting, filters, blocks,
   composer rules, display modes. Unit tests with the Python version's behaviour as
   reference (same inputs → same outputs; `tools/make_fixtures.py` records the Python
   functions' outputs, `crates/cbs-core/tests/python_parity.rs` compares).
2. ✅ **`cbs-osc`**: OSC, OSCQuery, VRChat finder (own small mDNS: like python-zeroconf
   it announces 127.0.0.1 on every interface; checked against python-zeroconf both ways).
3. **`cbs-audio`**: microphone, VAD, segmenter.
4. **`cbs-asr`**: CTranslate2 Whisper + NLLB, whisper.cpp client, Qwen; GPU helper process.
5. **Headless app**: microphone → chatbox end to end, without a window.
6. **`cbs-sys` + `cbs-media`**: all live values, VR, devices, music, lyrics.
7. **GUI**: main window, routes, transcript, settings, chatbox editor, Earshot tab, graphics
   panel, help, themes.
8. **Setup** and **packages** (Windows setup .exe, .deb, .rpm, Arch, ZIP), CI.
9. **Parity pass** through this whole list; then the Python repo points to the Rust one.

## 25. Open questions and risks

- **CTranslate2 builds**: CUDA and ROCm shared libraries have to be built in CI for Linux and
  Windows; ROCm on Windows is the least certain. Fallback: whisper.cpp with its CUDA / HIP
  backends.
- **OpenVR** from Rust: the bindings are old; it may be simpler to load `libopenvr_api`
  directly.
- **Windows-only APIs** (SMTC, Core Audio, PDH, DXGI) can't be tested on this machine; CI
  can build them, real testing needs a Windows PC.
- **Monado / WiVRn**: only testable with a headset; the D-Bus and libmonado calls are ported
  1:1.
- **Font metrics** of the chatbox preview must match the Python version (same Noto fonts,
  same measured widths).

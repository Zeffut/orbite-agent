// The Conversation in a browser: what Orbite shows is listened to as it
// comes, and what the Owner writes is sent as it is.
(() => {
  // On a server that holds several agents, what is asked of an agent is
  // asked of the one chosen here; what is the server's is asked of it.
  const plain = window.fetch.bind(window);
  const place = { agents: null, chosen: "", plain };
  window.orbite = place;
  const at = (path) =>
    place.chosen && typeof path === "string" && path.startsWith("/api/") && !/^\/api\/(pair|agents)\b/.test(path)
      ? "/a/" + place.chosen + path
      : path;
  window.fetch = (path, options) => plain(at(path), options);
  // Which agents this place holds, and which of them this device talks to:
  // the one it last chose, while it is still there.
  async function find() {
    place.agents = null;
    place.chosen = "";
    const held = await plain("/api/agents").catch(() => null);
    if (!held || !held.ok) return;
    place.agents = (await held.json().catch(() => ({}))).agents || [];
    let kept = "";
    try { kept = localStorage.getItem("agent") || ""; } catch (_) { /* a browser that keeps nothing */ }
    const chosen = place.agents.find((agent) => agent.id === kept) || place.agents[0];
    place.chosen = chosen ? chosen.id : "";
  }

  const WORDS = {
    en: {
      pairTitle: "Let this device in",
      pairHow: "On the machine where Orbite runs, ask for a code, then type it here. It works once, for ten minutes.",
      pairCode: "Code", pairName: "Name of this device", pairGo: "Let it in",
      pairWrong: "That code is not the right one, or it has run out. Ask for a fresh one.",
      write: "Write to your agent", send: "Send", offline: "Reconnecting…",
      allow: "Allow", always: "Always", deny: "Deny",
      allowed: "Allowed", alwaysAllowed: "Always allowed", denied: "Denied", wrote: "Left out: you wrote instead",
      undone: "This cannot be undone.", critical: "Asked every time, whatever your settings.",
      working: "Working…", elsewhere: "From another place", browser: "Browser",
      close: "Close", latest: "Latest messages",
      writeTo: "Message {name}", today: "Today", yesterday: "Yesterday",
      attach: "Attach a file", record: "Record a voice message", cancel: "Cancel", remove: "Remove",
      voice: "Voice message", tooBig: "That file is too big: 50 MB at most.",
      holdOn: "Keep it pressed to record.", letGo: "Release to send · slide left to cancel".replace(" · ", "\n"), letGoToCancel: "Release to cancel",
      notSent: "That was not sent. Try again.", fileGone: "A file did not go through: attach it again.",
      noMic: "I cannot reach the microphone: allow it for this page in your browser.",
    },
    fr: {
      pairTitle: "Autoriser cet appareil",
      pairHow: "Sur la machine où tourne Orbite, demande un code, puis saisis-le ici. Il sert une fois, pendant dix minutes.",
      pairCode: "Code", pairName: "Nom de cet appareil", pairGo: "Autoriser",
      pairWrong: "Ce code n'est pas le bon, ou il a expiré. Demandes-en un nouveau.",
      write: "Écris à ton agent", send: "Envoyer", offline: "Reconnexion…",
      allow: "Autoriser", always: "Toujours", deny: "Refuser",
      allowed: "Autorisé", alwaysAllowed: "Toujours autorisé", denied: "Refusé", wrote: "Laissé de côté : tu as écrit à la place",
      undone: "Cela ne peut pas être annulé.", critical: "Demandé à chaque fois, quels que soient tes réglages.",
      working: "Au travail…", elsewhere: "Depuis un autre endroit", browser: "Navigateur",
      close: "Fermer", latest: "Derniers messages",
      writeTo: "Envoyer un message à {name}", today: "Aujourd'hui", yesterday: "Hier",
      attach: "Joindre un fichier", record: "Enregistrer un message vocal", cancel: "Annuler", remove: "Retirer",
      voice: "Message vocal", tooBig: "Ce fichier est trop gros : 50 Mo au plus.",
      holdOn: "Reste appuyé pour enregistrer.", letGo: "Relâche pour envoyer\nGlisse à gauche pour annuler", letGoToCancel: "Relâche pour annuler",
      notSent: "Ce n'est pas parti. Réessaie.", fileGone: "Un fichier n'est pas passé : joins-le à nouveau.",
      noMic: "Je n'ai pas accès au micro : autorise-le pour cette page dans ton navigateur.",
    },
  };
  let words = WORDS.en;
  const $ = (id) => document.getElementById(id);
  const make = (tag, className, text) => {
    const made = document.createElement(tag);
    if (className) made.className = className;
    if (text !== undefined) made.textContent = text;
    return made;
  };
  const post = (path, body) =>
    fetch(path, { method: "POST", headers: { "content-type": "application/json", "x-orbite": "1" }, body: JSON.stringify(body) });

  function speak(language) {
    words = WORDS[language] || WORDS.en;
    document.documentElement.lang = WORDS[language] ? language : "en";
    for (const said of document.querySelectorAll("[data-say]")) said.textContent = words[said.dataset.say];
    for (const field of document.querySelectorAll("[data-placeholder]")) field.placeholder = words[field.dataset.placeholder];
    for (const named of document.querySelectorAll("[data-label]")) named.setAttribute("aria-label", words[named.dataset.label]);
  }

  // What the Agent writes, with the few marks it uses shown as they are meant:
  // built piece by piece, so that nothing it wrote is ever read as a page.
  function written(text) {
    const whole = document.createDocumentFragment();
    const blocks = text.split(/```(?:[a-z]*\n)?/);
    blocks.forEach((block, at) => {
      if (at % 2 === 1) {
        const shown = make("pre");
        shown.append(make("code", "", block.replace(/\n$/, "")));
        whole.append(shown);
        return;
      }
      // A run of lines that each begin as an item is a list; a line that
      // begins with # is a title; the rest is written as it is.
      const lines = block.split("\n");
      let list = null;
      lines.forEach((line, place) => {
        const item = line.match(/^\s*(?:[-*•]|(\d+)[.)])\s+(.*)$/);
        const title = line.match(/^#{1,4}\s+(.*)$/);
        if (item) {
          const kind = item[1] ? "ol" : "ul";
          if (!list || list.localName !== kind) {
            list = make(kind);
            whole.append(list);
          }
          const one = make("li");
          inline(item[2], one);
          list.append(one);
          return;
        }
        list = null;
        if (title) {
          const heading = make("strong", "title");
          inline(title[1], heading);
          whole.append(heading);
        } else {
          inline(line, whole);
        }
        // The line's end, unless a list or the block's end follows.
        const next = lines[place + 1];
        if (next !== undefined && !/^\s*(?:[-*•]|\d+[.)])\s+/.test(next)) whole.append("\n");
      });
    });
    return whole;
  }

  // What is written within a line: bold, code, links.
  function inline(line, into) {
    const marks = /(\*\*[^*\n]+\*\*|`[^`\n]+`|\[[^\]\n]+\]\(https?:\/\/[^\s)]+\)|https?:\/\/[^\s<>()]+[^\s<>().,;:!?])/g;
    let from = 0;
    for (const found of line.matchAll(marks)) {
      into.append(line.slice(from, found.index));
      const mark = found[0];
      if (mark.startsWith("**")) into.append(make("strong", "", mark.slice(2, -2)));
      else if (mark.startsWith("`")) into.append(make("code", "", mark.slice(1, -1)));
      else {
        const named = mark.match(/^\[([^\]]+)\]\((.+)\)$/);
        const link = make("a", "", named ? named[1] : mark);
        link.href = named ? named[2] : mark;
        link.target = "_blank";
        link.rel = "noopener noreferrer";
        // Opened by the page itself: a browser makes a tab of it, and
        // Orbite's app, which shows no other site, hands it to the browser.
        link.addEventListener("click", (clicked) => {
          if (clicked.ctrlKey || clicked.metaKey || clicked.shiftKey || clicked.button !== 0) return;
          clicked.preventDefault();
          window.open(link.href, "_blank", "noopener,noreferrer");
        });
        into.append(link);
      }
      from = found.index + mark.length;
    }
    into.append(line.slice(from));
  }

  const said = $("said");
  const atTheEnd = () => said.scrollHeight - said.scrollTop - said.clientHeight < 120;
  // The hour is marked where the Conversation picks up after a while, or on another day.
  let lastAt = 0;
  function hour(at) {
    const then = new Date(at * 1000);
    const days = Math.round((new Date().setHours(0, 0, 0, 0) - new Date(then).setHours(0, 0, 0, 0)) / 86400000);
    const day = days === 0 ? words.today : days === 1 ? words.yesterday
      : then.toLocaleDateString([], { day: "numeric", month: "short" });
    return `${day} ${then.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  }
  function add(element, event) {
    const follow = atTheEnd();
    if (event.id) element.dataset.event = event.id;
    if (event.at && (event.at - lastAt > 900 || new Date(event.at * 1000).getDate() !== new Date(lastAt * 1000).getDate())) {
      said.append(make("div", "when", hour(event.at)));
    }
    if (event.at) lastAt = event.at;
    said.append(element);
    if (follow) said.scrollTop = said.scrollHeight;
  }
  // The Agent's mark stirs while it works, and a way back down shows when the Owner reads further up.
  const mascot = document.querySelector("#who .mascot");
  const working = (busy) => mascot.classList.toggle("working", busy);
  const waiting = () => mascot.classList.toggle("waiting", asks.size > 0);
  said.addEventListener("scroll", () => { $("down").hidden = atTheEnd(); });
  $("down").addEventListener("click", () => { said.scrollTop = said.scrollHeight; });

  // What came while the Owner was looking elsewhere is counted in the page's
  // title, with the latest in a few words: a tab shows it, and Orbite's app
  // reads it there to tell them.
  let agentName = "Orbite";
  let unseen = 0;
  function name(latest) {
    const few = (latest || "").replace(/\s+/g, " ").trim().slice(0, 90);
    document.title = unseen ? `(${unseen}) ${agentName}${few ? " — " + few : ""}` : agentName;
  }
  function came(latest) {
    if (document.hasFocus() && !document.hidden) return;
    unseen += 1;
    name(latest);
  }
  const looked = () => { if (unseen) { unseen = 0; name(); } };
  window.addEventListener("focus", looked);
  document.addEventListener("visibilitychange", () => { if (!document.hidden && document.hasFocus()) looked(); });

  const asks = new Map();
  let replaying = true;
  function show(event) {
    // Only what arrives now is new: not what a device finds again on opening.
    if (!replaying && event.id) {
      if (event.kind === "agent") came(event.text);
      else if (event.kind === "ask") came(event.summary);
      else if (event.kind === "file") came(event.caption || event.name);
    }
    switch (event.kind) {
      case "owner": {
        const bubble = make("div", "said owner" + (event.elsewhere ? " elsewhere" : ""));
        if (event.files && event.files.length) {
          const files = make("div", "files");
          for (const file of event.files) {
            const where = at("/api/file?id=" + encodeURIComponent(file.file));
            if (file.image) {
              const picture = make("img");
              picture.src = where;
              picture.alt = file.name;
              const follow = atTheEnd();
              picture.addEventListener("load", () => { if (follow) said.scrollTop = said.scrollHeight; });
              files.append(picture);
            } else {
              const spoken = /^voice-/.test(file.name);
              const link = make("a", spoken ? "doc voice" : "doc", spoken ? words.voice : file.name);
              link.href = where;
              link.download = file.name;
              files.append(link);
            }
          }
          bubble.append(files);
        }
        if (event.text) bubble.append(written(event.text));
        if (event.elsewhere) bubble.title = words.elsewhere;
        add(bubble, event);
        break;
      }
      case "sign-in": {
        // A service to sign in to: one touch, in a tab of its own, and
        // back here once it is done.
        const card = make("div", "choices");
        const asked = make("div", "asked", event.label);
        const go = make("a", "", document.documentElement.lang === "fr" ? "Se connecter" : "Sign in");
        go.href = event.link;
        go.target = "_blank";
        go.rel = "noopener";
        go.style.cssText = "display:block;text-align:center;padding:10px 14px;border-radius:12px;background:#f2f2f0;color:#0d0d0d;font-weight:600;text-decoration:none;margin-top:8px";
        card.append(asked, go);
        add(card, event);
        break;
      }
      case "agent": {
        if (event.choices && event.choices.length) {
          // A question and what can be answered, as one card.
          const choices = make("div", "choices");
          if (event.text) {
            const asked = make("div", "asked");
            asked.append(written(event.text));
            choices.append(asked);
          }
          event.choices.forEach((choice, place) => {
            const button = make("button");
            button.type = "button";
            button.append(make("span", "letter", String.fromCharCode(65 + (place % 26))), make("span", "", choice.label));
            button.addEventListener("click", () => {
              for (const other of choices.querySelectorAll("button")) other.disabled = true;
              post("/api/say", { text: choice.command, tapped: true });
            });
            choices.append(button);
          });
          add(choices, event);
        } else if (event.text) {
          const bubble = make("div", "said agent");
          bubble.append(written(event.text));
          // A link that lets a device in, as a code to point a phone at.
          if (event.code) {
            const code = make("img", "code");
            code.alt = "";
            code.src = "data:image/svg+xml," + encodeURIComponent(event.code);
            bubble.append(code);
          }
          add(bubble, event);
        }
        $("doing").hidden = true;
        working(false);
        break;
      }
      case "ask": {
        const card = make("div", "ask" + (event.critical ? " critical" : ""));
        card.append(make("strong", "", event.summary));
        if (event.detail) card.append(make("p", "detail", event.detail));
        if (event.irreversible) card.append(make("p", "note", words.undone));
        if (event.critical) card.append(make("p", "note", words.critical));
        const answers = make("div", "answers");
        const offered = event.critical ? ["allow", "deny"] : ["allow", "always", "deny"];
        for (const answer of offered) {
          const button = make("button", answer, words[answer]);
          button.type = "button";
          button.addEventListener("click", () => {
            for (const other of answers.querySelectorAll("button")) other.disabled = true;
            post("/api/answer", { ask: event.ask, answer });
          });
          answers.append(button);
        }
        card.append(answers);
        asks.set(event.ask, card);
        add(card, event);
        waiting();
        break;
      }
      case "settled": {
        const card = asks.get(event.ask);
        if (!card) break;
        // Once answered, the card gives way to a line: what was asked, and what was said of it.
        const told = { allow: words.allowed, always: words.alwaysAllowed, deny: words.denied, wrote: words.wrote }[event.answer];
        const how = event.answer === "allow" || event.answer === "always" ? " allowed" : "";
        const line = make("div", "event answered" + how);
        line.append(make("strong", "", card.querySelector("strong").textContent), ` · ${told}`);
        const detail = card.querySelector(".detail");
        if (detail) line.title = detail.textContent;
        if (card.dataset.event) line.dataset.event = card.dataset.event;
        card.replaceWith(line);
        asks.delete(event.ask);
        waiting();
        break;
      }
      case "file": {
        const shown = make("div", "file");
        const where = at("/api/file?id=" + encodeURIComponent(event.file));
        if (event.image) {
          const picture = make("img");
          picture.src = where;
          picture.alt = event.caption || event.name;
          picture.loading = "lazy";
          const follow = atTheEnd();
          picture.addEventListener("load", () => { if (follow) said.scrollTop = said.scrollHeight; });
          const open = make("a");
          open.href = where;
          open.target = "_blank";
          open.rel = "noopener";
          open.append(picture);
          shown.append(open);
        } else {
          const link = make("a", "doc", event.name);
          link.href = where;
          link.download = event.name;
          shown.append(link);
        }
        if (event.caption) shown.append(make("span", "", event.caption));
        add(shown, event);
        break;
      }
      case "erase":
        for (const gone of event.events || []) {
          const there = said.querySelector(`[data-event="${Number(gone)}"]`);
          if (there) there.remove();
        }
        break;
      case "caught-up":
        replaying = false;
        break;
      case "step":
        $("doing").textContent = `${event.icon} ${event.text}`;
        $("doing").hidden = false;
        working(true);
        break;
      case "working":
        if ($("doing").hidden) {
          $("doing").textContent = words.working;
          $("doing").hidden = false;
        }
        working(true);
        break;
      case "idle":
        $("doing").hidden = true;
        working(false);
        break;
    }
  }

  let listening;
  function listen() {
    listening = new EventSource(at("/api/events"));
    listening.onopen = () => { $("link").hidden = true; };
    listening.onmessage = (sent) => {
      const event = JSON.parse(sent.data);
      if (sent.lastEventId) event.id = sent.lastEventId;
      show(event);
    };
    listening.onerror = async () => {
      $("link").hidden = false;
      // Turned away, or never let in: back to pairing.
      const me = await fetch("/api/me").catch(() => null);
      if (me && me.status === 401) {
        listening.close();
        start();
      }
    };
  }

  let noting;
  function note(told) {
    $("note").textContent = told;
    $("note").hidden = false;
    clearTimeout(noting);
    noting = setTimeout(() => { $("note").hidden = true; }, 6000);
  }

  // Files go ahead of the message they are sent with, and wait under what is
  // being written until it is sent or they are taken back.
  const attached = [];
  const canRecord = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia && window.MediaRecorder);
  const text = $("text");
  function arrange() {
    $("attached").hidden = attached.length === 0;
    const nothing = !text.value.trim() && attached.length === 0;
    $("mic").hidden = !(canRecord && nothing);
    $("send").hidden = canRecord && nothing;
  }
  function sendAhead(file, name) {
    return fetch("/api/upload?name=" + encodeURIComponent(name), { method: "POST", headers: { "x-orbite": "1" }, body: file })
      .then((answer) => (answer.ok ? answer.json() : Promise.reject(answer.status)))
      .then((taken) => taken.file);
  }
  const takeBack = (token) => fetch("/api/upload?file=" + encodeURIComponent(token), { method: "DELETE", headers: { "x-orbite": "1" } }).catch(() => {});
  function attach(file) {
    if (file.size > 50000000) return note(words.tooBig);
    const chip = make("div", "chip sending");
    if (/^image\//.test(file.type)) {
      const thumb = make("img");
      thumb.src = URL.createObjectURL(file);
      thumb.alt = "";
      thumb.addEventListener("load", () => URL.revokeObjectURL(thumb.src));
      chip.append(thumb);
    }
    chip.append(make("span", "", file.name));
    const remove = make("button", "", "✕");
    remove.type = "button";
    remove.setAttribute("aria-label", words.remove);
    chip.append(remove);
    const one = { chip, token: null, gone: false };
    one.sent = sendAhead(file, file.name || "file").then(
      (token) => { one.token = token; chip.classList.remove("sending"); if (one.gone) takeBack(token); },
      () => { chip.classList.remove("sending"); chip.classList.add("failed"); },
    );
    remove.addEventListener("click", () => {
      one.gone = true;
      if (one.token) takeBack(one.token);
      attached.splice(attached.indexOf(one), 1);
      chip.remove();
      arrange();
    });
    attached.push(one);
    $("attached").append(chip);
    arrange();
  }
  $("files").addEventListener("change", () => {
    for (const file of $("files").files) attach(file);
    $("files").value = "";
    text.focus();
  });
  $("attach").addEventListener("keydown", (pressed) => {
    if (pressed.key === "Enter" || pressed.key === " ") { pressed.preventDefault(); $("files").click(); }
  });
  text.addEventListener("paste", (pasted) => {
    const files = [...(pasted.clipboardData ? pasted.clipboardData.files : [])];
    if (!files.length) return;
    pasted.preventDefault();
    files.forEach(attach);
  });
  const carriesFiles = (moved) => moved.dataTransfer && [...moved.dataTransfer.types].includes("Files");
  $("talk").addEventListener("dragover", (moved) => {
    if (!carriesFiles(moved)) return;
    moved.preventDefault();
    $("talk").classList.add("dropping");
  });
  $("talk").addEventListener("dragleave", (moved) => {
    if (!$("talk").contains(moved.relatedTarget)) $("talk").classList.remove("dropping");
  });
  $("talk").addEventListener("drop", (moved) => {
    if (!carriesFiles(moved)) return;
    moved.preventDefault();
    $("talk").classList.remove("dropping");
    [...moved.dataTransfer.files].forEach(attach);
  });

  const fit = () => { text.style.height = "auto"; text.style.height = Math.min(text.scrollHeight, window.innerHeight * 0.4) + "px"; arrange(); };
  text.addEventListener("input", fit);
  text.addEventListener("keydown", (pressed) => {
    if (pressed.key === "Enter" && !pressed.shiftKey && !pressed.isComposing) {
      pressed.preventDefault();
      $("say").requestSubmit();
    }
  });
  let sending = false;
  $("say").addEventListener("submit", async (sent) => {
    sent.preventDefault();
    if (sending) return;
    const saying = text.value.trim();
    const going = attached.slice();
    if (!saying && !going.length) return;
    sending = true;
    $("send").disabled = true;
    text.value = "";
    fit();
    await Promise.all(going.map((one) => one.sent));
    const files = going.filter((one) => one.token).map((one) => one.token);
    const failed = going.some((one) => !one.token);
    const answer = failed ? null : await post("/api/say", { text: saying, files }).catch(() => null);
    sending = false;
    $("send").disabled = false;
    if (answer && answer.ok) {
      for (const one of going) {
        attached.splice(attached.indexOf(one), 1);
        one.chip.remove();
      }
      said.scrollTop = said.scrollHeight;
    } else {
      // Not sent: what was written is given back to be sent again, with
      // the files that did go through.
      if (!text.value) text.value = saying;
      const refused = answer && answer.status === 410;
      for (const one of going) {
        if (one.token && !refused) continue;
        attached.splice(attached.indexOf(one), 1);
        one.chip.remove();
      }
      note(failed || refused ? words.fileGone : words.notSent);
    }
    fit();
  });

  // A voice message: recorded here, sent as a file the Agent has written down.
  let recorder, recorded, ticking, keeping;
  function stopRecording(keep) {
    keeping = keep;
    if (recorder && recorder.state !== "inactive") recorder.stop();
  }
  // Held down: it records while the finger stays, sends when it lifts,
  // and gives up when it slides away. From a keyboard, where nothing is
  // held, it starts and the bar beside it sends or gives up.
  let holding = null;
  async function record(held) {
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch (refused) {
      return note(words.noMic);
    }
    // Let go while the microphone was being asked for: nothing to record.
    if (held && holding !== held) {
      for (const track of stream.getTracks()) track.stop();
      return;
    }
    // Safari, and every browser on an iPhone, records best in its own
    // wrapping: what it makes in the others' does not play back whole.
    const apple = /AppleWebKit/.test(navigator.userAgent) && !/Chrome|Chromium|Edg\//.test(navigator.userAgent);
    const usual = [["audio/ogg;codecs=opus", "ogg"], ["audio/webm;codecs=opus", "webm"], ["audio/mp4", "m4a"]];
    const kinds = apple ? [usual[2], usual[1], usual[0]] : usual;
    const [kind, ending] = kinds.find(([kind]) => MediaRecorder.isTypeSupported(kind)) || ["", "webm"];
    recorder = new MediaRecorder(stream, kind ? { mimeType: kind } : undefined);
    recorded = [];
    const began = Date.now();
    if (held) held.began = began;
    recorder.addEventListener("dataavailable", (part) => { if (part.data.size) recorded.push(part.data); });
    recorder.addEventListener("stop", async () => {
      clearInterval(ticking);
      for (const track of stream.getTracks()) track.stop();
      $("recording").hidden = true;
      $("say").hidden = false;
      $("say").classList.remove("holding", "letting-go");
      $("held").hidden = true;
      if (!keeping || !recorded.length) return;
      const spoken = new Blob(recorded, { type: recorder.mimeType || kind });
      try {
        const token = await sendAhead(spoken, `voice-${began}.${ending}`);
        const answer = await post("/api/say", { text: "", files: [token] });
        if (!answer.ok) throw answer.status;
        said.scrollTop = said.scrollHeight;
      } catch (failed) {
        note(words.notSent);
      }
    });
    const shown = () => {
      const seconds = Math.floor((Date.now() - began) / 1000);
      const time = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
      $("recordTime").textContent = time;
      $("heldTime").textContent = time;
    };
    shown();
    ticking = setInterval(shown, 500);
    if (held) {
      $("note").hidden = true;
      $("heldHint").textContent = words.letGo;
      $("held").hidden = false;
      $("say").classList.add("holding");
      if (navigator.vibrate) navigator.vibrate(12);
    } else {
      $("say").hidden = true;
      $("recording").hidden = false;
    }
    recorder.start();
  }
  const mic = $("mic");
  mic.addEventListener("pointerdown", (pressed) => {
    if (pressed.button) return;
    pressed.preventDefault();
    mic.setPointerCapture(pressed.pointerId);
    holding = { from: pressed.clientX, at: Date.now(), away: false };
    record(holding);
  });
  mic.addEventListener("pointermove", (moved) => {
    if (!holding) return;
    holding.away = holding.from - moved.clientX > 70;
    $("say").classList.toggle("letting-go", holding.away);
    $("heldHint").textContent = holding.away ? words.letGoToCancel : words.letGo;
  });
  const letGo = (given_up) => {
    if (!holding) return;
    const held = holding;
    holding = null;
    // A touch too short to say anything is a question: how is this used?
    const long_enough = held.began && Date.now() - held.began > 500;
    if (!long_enough && !given_up) note(words.holdOn);
    stopRecording(long_enough && !held.away && !given_up);
  };
  mic.addEventListener("pointerup", () => letGo(false));
  mic.addEventListener("pointercancel", () => letGo(true));
  mic.addEventListener("contextmenu", (asked) => asked.preventDefault());
  // From a keyboard: started here, sent or given up from the bar.
  mic.addEventListener("click", (pressed) => {
    if (pressed.detail === 0 && !holding) record(null);
  });
  $("recordStop").addEventListener("click", () => stopRecording(false));
  $("recordSend").addEventListener("click", () => stopRecording(true));

  $("pair").addEventListener("submit", async (sent) => {
    sent.preventDefault();
    $("pairWrong").hidden = true;
    const answer = await post("/api/pair", { code: $("code").value, name: $("device").value });
    if (answer.ok) start();
    else $("pairWrong").hidden = false;
  });

  function deviceName() {
    const agent = navigator.userAgent;
    const what = /iPhone/.test(agent) ? "iPhone" : /iPad/.test(agent) ? "iPad" : /Android/.test(agent) ? "Android"
      : /Mac/.test(agent) ? "Mac" : /Windows/.test(agent) ? "Windows" : /Linux/.test(agent) ? "Linux" : words.browser;
    const which = /Firefox/.test(agent) ? "Firefox" : /Edg\//.test(agent) ? "Edge" : /Chrome/.test(agent) ? "Chrome" : /Safari/.test(agent) ? "Safari" : "";
    return which ? `${what} · ${which}` : what;
  }

  // The Agent as its Owner chose it, in place of Orbite's own planet.
  function dress(look) {
    const drawn = (name, given) => {
      const part = document.createElementNS("http://www.w3.org/2000/svg", name);
      for (const [key, value] of Object.entries(given)) part.setAttribute(key, value);
      return part;
    };
    const eyes = drawn("g", { class: "eyes" });
    for (const x of [38, 55]) eyes.append(drawn("rect", { x, y: look.eyes, width: 7, height: 15, rx: 3.5 }));
    mascot.classList.add("agent");
    mascot.style.setProperty("--look", look.colour);
    mascot.replaceChildren(
      drawn("path", { class: "body", d: look.outline }),
      eyes,
      drawn("circle", { class: "dot", cx: 84, cy: 18, r: 10 }),
    );
  }

  async function start() {
    await find();
    // A server with no agent yet: the first one is made here, by its name.
    if (place.agents && !place.agents.length) {
      const called = (window.prompt(words.firstAgent || "What is your first agent called?") || "").trim();
      if (called) {
        await plain("/api/agents", { method: "POST", headers: { "content-type": "application/json", "x-orbite": "1" }, body: JSON.stringify({ name: called }) }).catch(() => null);
        await find();
      }
    }
    const me = await fetch("/api/me");
    // An agent that is starting, or made anew for a new version, answers
    // in a moment: asked again rather than taken for a stranger's page.
    if (place.chosen && me.status >= 500) {
      $("link").hidden = false;
      // Its place is shown meanwhile: another agent can be gone over to.
      $("pairing").hidden = true;
      $("talk").hidden = false;
      return void setTimeout(start, 3000);
    }
    const told = await me.json().catch(() => ({}));
    speak(told.language);
    if (me.ok) {
      $("pairing").hidden = true;
      $("talk").hidden = false;
      said.replaceChildren();
      asks.clear();
      waiting();
      agentName = told.agent || "Orbite";
      if (told.look) dress(told.look);
      $("title").textContent = agentName;
      unseen = 0;
      name();
      text.placeholder = words.writeTo.replace("{name}", agentName);
      lastAt = 0;
      listen();
      arrange();
      text.focus();
    } else {
      $("talk").hidden = true;
      $("pairing").hidden = false;
      if (!$("device").value) $("device").value = deviceName();
      // Opened from the link Orbite gave: the code came with it, and is
      // taken out of the address so that it stays in no history.
      const given = decodeURIComponent(location.hash.slice(1));
      if (/^[A-Za-z0-9]{4}-[A-Za-z0-9]{4}$/.test(given)) {
        history.replaceState(null, "", location.pathname);
        $("code").value = given.toUpperCase();
        $("pair").requestSubmit();
        return;
      }
      $("code").focus();
    }
  }
  start();
})();

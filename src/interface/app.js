// The Conversation in a browser: what Orbite shows is listened to as it
// comes, and what the Owner writes is sent as it is.
(() => {
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
      attach: "Attach a file", record: "Record a voice message", cancel: "Cancel", remove: "Remove",
      voice: "Voice message", tooBig: "That file is too big: 50 MB at most.",
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
      attach: "Joindre un fichier", record: "Enregistrer un message vocal", cancel: "Annuler", remove: "Retirer",
      voice: "Message vocal", tooBig: "Ce fichier est trop gros : 50 Mo au plus.",
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
      const marks = /(\*\*[^*\n]+\*\*|`[^`\n]+`|\[[^\]\n]+\]\(https?:\/\/[^\s)]+\)|https?:\/\/[^\s<>()]+[^\s<>().,;:!?])/g;
      let from = 0;
      for (const found of block.matchAll(marks)) {
        whole.append(block.slice(from, found.index));
        const mark = found[0];
        if (mark.startsWith("**")) whole.append(make("strong", "", mark.slice(2, -2)));
        else if (mark.startsWith("`")) whole.append(make("code", "", mark.slice(1, -1)));
        else {
          const named = mark.match(/^\[([^\]]+)\]\((.+)\)$/);
          const link = make("a", "", named ? named[1] : mark);
          link.href = named ? named[2] : mark;
          link.target = "_blank";
          link.rel = "noopener noreferrer";
          whole.append(link);
        }
        from = found.index + mark.length;
      }
      whole.append(block.slice(from));
    });
    return whole;
  }

  const said = $("said");
  const atTheEnd = () => said.scrollHeight - said.scrollTop - said.clientHeight < 120;
  function add(element, event) {
    const follow = atTheEnd();
    if (event.id) element.dataset.event = event.id;
    said.append(element);
    if (follow) said.scrollTop = said.scrollHeight;
  }

  const asks = new Map();
  function show(event) {
    switch (event.kind) {
      case "owner": {
        const bubble = make("div", "said owner" + (event.elsewhere ? " elsewhere" : ""));
        if (event.files && event.files.length) {
          const files = make("div", "files");
          for (const file of event.files) {
            const where = "/api/file?id=" + encodeURIComponent(file.file);
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
      case "agent": {
        if (event.text) {
          const bubble = make("div", "said agent");
          bubble.append(written(event.text));
          add(bubble, event);
        }
        if (event.choices && event.choices.length) {
          const choices = make("div", "choices");
          for (const choice of event.choices) {
            const button = make("button", "", choice.label);
            button.type = "button";
            button.addEventListener("click", () => {
              for (const other of choices.querySelectorAll("button")) other.disabled = true;
              post("/api/say", { text: choice.command, tapped: true });
            });
            choices.append(button);
          }
          add(choices, event);
        }
        $("doing").hidden = true;
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
        break;
      }
      case "settled": {
        const card = asks.get(event.ask);
        if (!card) break;
        const answers = card.querySelector(".answers");
        if (answers) answers.remove();
        const told = { allow: words.allowed, always: words.alwaysAllowed, deny: words.denied, wrote: words.wrote }[event.answer];
        const how = event.answer === "allow" || event.answer === "always" ? " allowed" : "";
        card.append(make("div", "answered" + how, told));
        break;
      }
      case "file": {
        const shown = make("div", "file");
        const where = "/api/file?id=" + encodeURIComponent(event.file);
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
      case "step":
        $("doing").textContent = `${event.icon} ${event.text}`;
        $("doing").hidden = false;
        break;
      case "working":
        if ($("doing").hidden) {
          $("doing").textContent = words.working;
          $("doing").hidden = false;
        }
        break;
      case "idle":
        $("doing").hidden = true;
        break;
    }
  }

  let listening;
  function listen() {
    listening = new EventSource("/api/events");
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
  $("mic").addEventListener("click", async () => {
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch (refused) {
      return note(words.noMic);
    }
    const kinds = [["audio/ogg;codecs=opus", "ogg"], ["audio/webm;codecs=opus", "webm"], ["audio/mp4", "m4a"]];
    const [kind, ending] = kinds.find(([kind]) => MediaRecorder.isTypeSupported(kind)) || ["", "webm"];
    recorder = new MediaRecorder(stream, kind ? { mimeType: kind } : undefined);
    recorded = [];
    const began = Date.now();
    recorder.addEventListener("dataavailable", (part) => { if (part.data.size) recorded.push(part.data); });
    recorder.addEventListener("stop", async () => {
      clearInterval(ticking);
      for (const track of stream.getTracks()) track.stop();
      $("recording").hidden = true;
      $("say").hidden = false;
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
      $("recordTime").textContent = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
    };
    shown();
    ticking = setInterval(shown, 500);
    $("say").hidden = true;
    $("recording").hidden = false;
    recorder.start();
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

  async function start() {
    const me = await fetch("/api/me");
    const told = await me.json().catch(() => ({}));
    speak(told.language);
    if (me.ok) {
      $("pairing").hidden = true;
      $("talk").hidden = false;
      said.replaceChildren();
      asks.clear();
      listen();
      arrange();
      text.focus();
    } else {
      $("talk").hidden = true;
      $("pairing").hidden = false;
      if (!$("device").value) $("device").value = deviceName();
      $("code").focus();
    }
  }
  start();
})();

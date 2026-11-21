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
        bubble.append(written(event.text));
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

  const text = $("text");
  const fit = () => { text.style.height = "auto"; text.style.height = Math.min(text.scrollHeight, window.innerHeight * 0.4) + "px"; };
  text.addEventListener("input", fit);
  text.addEventListener("keydown", (pressed) => {
    if (pressed.key === "Enter" && !pressed.shiftKey && !pressed.isComposing) {
      pressed.preventDefault();
      $("say").requestSubmit();
    }
  });
  $("say").addEventListener("submit", async (sent) => {
    sent.preventDefault();
    const saying = text.value.trim();
    if (!saying) return;
    text.value = "";
    fit();
    said.scrollTop = said.scrollHeight;
    const answer = await post("/api/say", { text: saying }).catch(() => null);
    if (!answer || !answer.ok) {
      // Not sent: what was written is given back to be sent again.
      text.value = saying;
      fit();
    }
  });

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

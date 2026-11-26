// The page on a computer: a column beside the Conversation, with the Agent,
// what it is connected to and how it is set. A desk has room a phone has
// not: nothing here is a phone's screen stretched.
(() => {
  "use strict";
  const $ = (id) => document.getElementById(id);
  const make = (tag, className, text) => {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const wide = matchMedia("(min-width: 900px)");
  const side = $("side"), panel = $("panel"), talk = $("talk");
  let french = (navigator.language || "").startsWith("fr");
  const say = (english, inFrench) => (french ? inFrench : english);
  const post = (path, body) =>
    fetch(path, { method: "POST", headers: { "content-type": "application/json", "x-orbite": "1" }, body: JSON.stringify(body) });
  let me = null, kept = null, showing = "talk";

  // The Agent as it looks: its shape in its colour, and its two eyes.
  function figure(outline, colour, eyes, size) {
    const box = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    box.setAttribute("viewBox", "0 0 100 100");
    box.setAttribute("width", size);
    box.setAttribute("height", size);
    // Drawn only from what a look is made of: a path of points, a colour
    // written as one, a number. Another agent's look is its own to say.
    const at = Number.isFinite(Number(eyes)) ? Number(eyes) : 36;
    if (!/^[MLZ0-9 .,-]+$/.test(outline || "")) outline = "";
    if (!/^#[0-9a-fA-F]{3,8}$/.test(colour || "")) colour = "#f2f2f0";
    box.innerHTML = outline
      ? `<path d="${outline}" fill="${colour}"/><g fill="#0d0d0d"><rect x="38" y="${at}" width="7" height="15" rx="3.5"/><rect x="55" y="${at}" width="7" height="15" rx="3.5"/></g>`
      : `<g transform="rotate(-24 50 50)"><ellipse cx="50" cy="50" rx="44" ry="13" fill="none" stroke="currentColor" stroke-width="5"/></g><circle cx="50" cy="50" r="26" fill="currentColor"/><g fill="var(--ground)"><rect x="40" y="40" width="6" height="13" rx="3"/><rect x="54" y="40" width="6" height="13" rx="3"/></g>`;
    return box;
  }

  function column() {
    side.replaceChildren();
    side.append(make("div", "mark", "Orbite"));
    // Every agent, each its Conversation; what is set for all, at the bottom.
    side.append(make("small", "over", say("Agents", "Agents")));
    const mine = ["talk", "agent", "look", "screen"].includes(showing);
    const place = window.orbite || {};
    const one = (id, name, look, here) => {
      const who = make("button", "agent" + (here && mine ? " here" : ""));
      who.type = "button";
      who.append(figure(look && look.outline, look && look.colour, look && look.eyes, 34));
      const named = make("span");
      named.append(make("strong", "", name));
      who.append(named);
      who.onclick = () => (here ? open("talk") : go(id));
      side.append(who);
    };
    if (place.agents) {
      // Every agent of this server: the one talked to, then the others.
      for (const agent of place.agents) {
        const here = agent.id === place.chosen;
        one(agent.id, here ? (me && me.agent) || agent.name : (others[agent.id] || {}).agent || agent.name,
          here ? me && me.look : (others[agent.id] || {}).look, here);
      }
      const more = make("button", "new");
      more.type = "button";
      more.innerHTML = `<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M12 5v14M5 12h14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>`;
      more.append(make("span", "", say("New agent", "Nouvel agent")));
      more.onclick = () => {
        const name = make("input");
        name.placeholder = say("Its name", "Son nom");
        name.maxLength = 40;
        more.replaceWith(name);
        name.focus();
        name.onblur = () => { if (!name.value.trim()) column(); };
        name.onkeydown = async (pressed) => {
          if (pressed.key === "Escape") return column();
          if (pressed.key !== "Enter" || !name.value.trim() || name.disabled) return;
          name.disabled = true;
          const made = await post("/api/agents", { name: name.value.trim() }).catch(() => null);
          const agent = made && made.ok ? await made.json() : null;
          if (agent) return go(agent.id);
          name.disabled = false;
          name.value = "";
          name.placeholder = say("It could not be made", "Il n'a pas pu être créé");
        };
      };
      side.append(more);
    } else {
      one("", (me && me.agent) || "Orbite", me && me.look, true);
    }
    const set = make("button", "low" + (showing === "settings" || showing === "services" ? " here" : ""));
    set.type = "button";
    set.innerHTML = `<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM4 12h2M18 12h2M12 4v2M12 18v2M6.5 6.5l1.4 1.4M16.1 16.1l1.4 1.4M6.5 17.5l1.4-1.4M16.1 7.9l1.4-1.4" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
    set.append(make("span", "", say("Settings", "Réglages")));
    set.onclick = () => open("settings");
    side.append(set);
  }

  // Goes over to another agent of this server: the page starts again with it.
  function go(id) {
    try { localStorage.setItem("agent", id); } catch (_) { /* a browser that keeps nothing */ }
    location.reload();
  }
  // How the other agents of this server look and are called now.
  const others = {};
  async function meet() {
    const place = window.orbite || {};
    for (const agent of place.agents || []) {
      if (agent.id === place.chosen || others[agent.id]) continue;
      const told = await place.plain("/a/" + agent.id + "/api/me").catch(() => null);
      if (told && told.ok) others[agent.id] = await told.json();
    }
    column();
  }

  function open(place) {
    // The Computer is given back when its screen is left.
    if (showing === "screen" && place !== "screen") give();
    showing = place;
    column();
    panel.hidden = place === "talk";
    talk.classList.toggle("aside", place !== "talk");
    if (place === "talk") latest();
    if (place === "settings") settings();
    if (place === "services") services();
    if (place === "agent") agent();
    if (place === "look") look();
    if (place === "screen") screen();
  }

  // What starts a screen of the panel: where it is left from, then its title.
  function head(title, about, back) {
    panel.replaceChildren();
    const shut = make("button", "shut");
    shut.type = "button";
    shut.setAttribute("aria-label", say("Close", "Fermer"));
    shut.innerHTML = `<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`;
    shut.onclick = () => open("talk");
    panel.append(shut);
    if (back) {
      const to = make("button", "back");
      to.type = "button";
      to.innerHTML = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
      to.append(make("span", "", back[1]));
      to.onclick = () => open(back[0]);
      panel.append(to);
    }
    if (!title) return;
    const top = make("header");
    top.append(make("h1", "", title));
    if (about) top.append(make("p", "", about));
    panel.append(top);
  }

  async function who() {
    const told = await fetch("/api/me").catch(() => null);
    if (told && told.ok) {
      me = await told.json();
      french = me.language === "fr";
    }
    return me;
  }
  const agentName = () => (me && me.agent) || "Orbite";

  async function read() {
    const told = await fetch("/api/settings").catch(() => null);
    kept = told && told.ok ? await told.json() : null;
    return kept;
  }
  const value = (key) => ((kept && kept.settings.find((one) => one.key === key)) || {}).value || "";
  async function set(key, to) {
    const sent = await post("/api/settings", { key, value: to }).catch(() => null);
    return !!(sent && sent.ok);
  }

  // The marks of the services, from Simple Icons (CC0): each belongs to its
  // service, and is shown only to say which service is meant.
  const MARKS = {"airtable":["M11.992 1.966c-.434 0-.87.086-1.28.257L1.779 5.917c-.503.208-.49.908.012 1.116l8.982 3.558a3.266 3.266 0 0 0 2.454 0l8.982-3.558c.503-.196.503-.908.012-1.116l-8.957-3.694a3.255 3.255 0 0 0-1.272-.257zM23.4 8.056a.589.589 0 0 0-.222.045l-10.012 3.877a.612.612 0 0 0-.38.564v8.896a.6.6 0 0 0 .821.552L23.62 18.1a.583.583 0 0 0 .38-.551V8.653a.6.6 0 0 0-.6-.596zM.676 8.095a.644.644 0 0 0-.48.19C.086 8.396 0 8.53 0 8.69v8.355c0 .442.515.737.908.54l6.27-3.006.307-.147 2.969-1.436c.466-.22.43-.908-.061-1.092L.883 8.138a.57.57 0 0 0-.207-.044z","#18bfff"],"atlassian":["M7.12 11.084a.683.683 0 00-1.16.126L.075 22.974a.703.703 0 00.63 1.018h8.19a.678.678 0 00.63-.39c1.767-3.65.696-9.203-2.406-12.52zM11.434.386a15.515 15.515 0 00-.906 15.317l3.95 7.9a.703.703 0 00.628.388h8.19a.703.703 0 00.63-1.017L12.63.38a.664.664 0 00-1.196.006z","#0052cc"],"calendly":["M19.655 14.262c.281 0 .557.023.828.064 0 .005-.005.01-.005.014-.105.267-.234.534-.381.786l-1.219 2.106c-1.112 1.936-3.177 3.127-5.411 3.127h-2.432c-2.23 0-4.294-1.191-5.412-3.127l-1.218-2.106a6.251 6.251 0 0 1 0-6.252l1.218-2.106C6.736 4.832 8.8 3.641 11.035 3.641h2.432c2.23 0 4.294 1.191 5.411 3.127l1.219 2.106c.147.252.271.519.381.786 0 .004.005.009.005.014-.267.041-.543.064-.828.064-1.816 0-2.501-.607-3.291-1.306-.764-.676-1.711-1.517-3.44-1.517h-1.029c-1.251 0-2.387.455-3.2 1.278-.796.805-1.233 1.904-1.233 3.099v1.411c0 1.196.437 2.295 1.233 3.099.813.823 1.949 1.278 3.2 1.278h1.034c1.729 0 2.676-.841 3.439-1.517.791-.703 1.471-1.306 3.287-1.301Zm.005-3.237c.399 0 .794-.036 1.179-.11-.002-.004-.002-.01-.002-.014-.073-.414-.193-.823-.349-1.218.731-.12 1.407-.396 1.986-.819 0-.004-.005-.013-.005-.018-.331-1.085-.832-2.101-1.489-3.03-.649-.915-1.435-1.719-2.331-2.395-1.867-1.398-4.088-2.138-6.428-2.138-1.448 0-2.855.28-4.175.841-1.273.543-2.423 1.315-3.407 2.299S2.878 6.552 2.341 7.83c-.557 1.324-.842 2.726-.842 4.175 0 1.448.281 2.855.842 4.174.542 1.274 1.314 2.423 2.298 3.407s2.129 1.761 3.407 2.299c1.324.556 2.727.841 4.175.841 2.34 0 4.561-.74 6.428-2.137a10.815 10.815 0 0 0 2.331-2.396c.652-.929 1.158-1.949 1.489-3.03 0-.004.005-.014.005-.018-.579-.423-1.255-.699-1.986-.819.161-.395.276-.804.349-1.218.005-.009.005-.014.005-.023.869.166 1.692.506 2.404 1.035.685.505.552 1.075.446 1.416C22.184 20.437 17.619 24 12.221 24c-6.625 0-12-5.375-12-12s5.37-12 12-12c5.398 0 9.963 3.563 11.471 8.464.106.341.239.915-.446 1.421-.717.529-1.535.873-2.404 1.034.128.716.128 1.45 0 2.166-.387-.074-.782-.11-1.182-.11-4.184 0-3.968 2.823-6.736 2.823h-1.029c-1.899 0-3.15-1.357-3.15-3.095v-1.411c0-1.738 1.251-3.094 3.15-3.094h1.034c2.768 0 2.552 2.823 6.731 2.827Z","#006bff"],"clickup":["M2 18.439l3.69-2.828c1.961 2.56 4.044 3.739 6.363 3.739 2.307 0 4.33-1.166 6.203-3.704L22 18.405C19.298 22.065 15.941 24 12.053 24 8.178 24 4.788 22.078 2 18.439zM12.04 6.15l-6.568 5.66-3.036-3.52L12.055 0l9.543 8.296-3.05 3.509z","#7b68ee"],"cloudflare":["M16.5088 16.8447c.1475-.5068.0908-.9707-.1553-1.3154-.2246-.3164-.6045-.499-1.0615-.5205l-8.6592-.1123a.1559.1559 0 0 1-.1333-.0713c-.0283-.042-.0351-.0986-.021-.1553.0278-.084.1123-.1484.2036-.1562l8.7359-.1123c1.0351-.0489 2.1601-.8868 2.5537-1.9136l.499-1.3013c.0215-.0561.0293-.1128.0147-.168-.5625-2.5463-2.835-4.4453-5.5499-4.4453-2.5039 0-4.6284 1.6177-5.3876 3.8614-.4927-.3658-1.1187-.5625-1.794-.499-1.2026.119-2.1665 1.083-2.2861 2.2856-.0283.31-.0069.6128.0635.894C1.5683 13.171 0 14.7754 0 16.752c0 .1748.0142.3515.0352.5273.0141.083.0844.1475.1689.1475h15.9814c.0909 0 .1758-.0645.2032-.1553l.12-.4268zm2.7568-5.5634c-.0771 0-.1611 0-.2383.0112-.0566 0-.1054.0415-.127.0976l-.3378 1.1744c-.1475.5068-.0918.9707.1543 1.3164.2256.3164.6055.498 1.0625.5195l1.8437.1133c.0557 0 .1055.0263.1329.0703.0283.043.0351.1074.0214.1562-.0283.084-.1132.1485-.204.1553l-1.921.1123c-1.041.0488-2.1582.8867-2.5527 1.914l-.1406.3585c-.0283.0713.0215.1416.0986.1416h6.5977c.0771 0 .1474-.0489.169-.126.1122-.4082.1757-.837.1757-1.2803 0-2.6025-2.125-4.727-4.7344-4.727","#f38020"],"linear":["M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z","#5e6ad2"],"notion":["M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L17.86 1.968c-.42-.326-.981-.7-2.055-.607L3.01 2.295c-.466.046-.56.28-.374.466zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.935-.56.935-1.167V6.354c0-.606-.233-.933-.748-.887l-15.177.887c-.56.047-.747.327-.747.933zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.933l-4.577-7.186v6.952L12.21 19s0 .84-1.168.84l-3.222.186c-.093-.186 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.139c-.093-.514.28-.887.747-.933zM1.936 1.035l13.31-.98c1.634-.14 2.055-.047 3.082.7l4.249 2.986c.7.513.934.653.934 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.934c-.98.047-1.448-.093-1.962-.747l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V2.667c0-.839.374-1.54 1.447-1.632z","#111111"],"paypal":["M7.016 19.198h-4.2a.562.562 0 0 1-.555-.65L5.093.584A.692.692 0 0 1 5.776 0h7.222c3.417 0 5.904 2.488 5.846 5.5-.006.25-.027.5-.066.747A6.794 6.794 0 0 1 12.071 12H8.743a.69.69 0 0 0-.682.583l-.325 2.056-.013.083-.692 4.39-.015.087zM19.79 6.142c-.01.087-.01.175-.023.261a7.76 7.76 0 0 1-7.695 6.598H9.007l-.283 1.795-.013.083-.692 4.39-.134.843-.014.088H6.86l-.497 3.15a.562.562 0 0 0 .555.65h3.612c.34 0 .63-.249.683-.585l.952-6.031a.692.692 0 0 1 .683-.584h2.126a6.793 6.793 0 0 0 6.707-5.752c.306-1.95-.466-3.744-1.89-4.906z","#003087"],"sentry":["M13.91 2.505c-.873-1.448-2.972-1.448-3.844 0L6.904 7.92a15.478 15.478 0 0 1 8.53 12.811h-2.221A13.301 13.301 0 0 0 5.784 9.814l-2.926 5.06a7.65 7.65 0 0 1 4.435 5.848H2.194a.365.365 0 0 1-.298-.534l1.413-2.402a5.16 5.16 0 0 0-1.614-.913L.296 19.275a2.182 2.182 0 0 0 .812 2.999 2.24 2.24 0 0 0 1.086.288h6.983a9.322 9.322 0 0 0-3.845-8.318l1.11-1.922a11.47 11.47 0 0 1 4.95 10.24h5.915a17.242 17.242 0 0 0-7.885-15.28l2.244-3.845a.37.37 0 0 1 .504-.13c.255.14 9.75 16.708 9.928 16.9a.365.365 0 0 1-.327.543h-2.287c.029.612.029 1.223 0 1.831h2.297a2.206 2.206 0 0 0 1.922-3.31z","#362d59"],"stripe":["M13.976 9.15c-2.172-.806-3.356-1.426-3.356-2.409 0-.831.683-1.305 1.901-1.305 2.227 0 4.515.858 6.09 1.631l.89-5.494C18.252.975 15.697 0 12.165 0 9.667 0 7.589.654 6.104 1.872 4.56 3.147 3.757 4.992 3.757 7.218c0 4.039 2.467 5.76 6.476 7.219 2.585.92 3.445 1.574 3.445 2.583 0 .98-.84 1.545-2.354 1.545-1.875 0-4.965-.921-6.99-2.109l-.9 5.555C5.175 22.99 8.385 24 11.714 24c2.641 0 4.843-.624 6.328-1.813 1.664-1.305 2.525-3.236 2.525-5.732 0-4.128-2.524-5.851-6.594-7.305h.003z","#635bff"],"supabase":["M11.9 1.036c-.015-.986-1.26-1.41-1.874-.637L.764 12.05C-.33 13.427.65 15.455 2.409 15.455h9.579l.113 7.51c.014.985 1.259 1.408 1.873.636l9.262-11.653c1.093-1.375.113-3.403-1.645-3.403h-9.642z","#2fae76"],"trello":["M21.147 0H2.853A2.86 2.86 0 000 2.853v18.294A2.86 2.86 0 002.853 24h18.294A2.86 2.86 0 0024 21.147V2.853A2.86 2.86 0 0021.147 0zM10.34 17.287a.953.953 0 01-.953.953h-4a.954.954 0 01-.954-.953V5.38a.953.953 0 01.954-.953h4a.954.954 0 01.953.953zm9.233-5.467a.944.944 0 01-.953.947h-4a.947.947 0 01-.953-.947V5.38a.953.953 0 01.953-.953h4a.954.954 0 01.953.953z","#0052cc"]};

  // What a value is called for a person.
  const WORDS = {
    careful: ["Careful", "Prudent"], balanced: ["Balanced", "Équilibré"], free: ["Free", "Libre"],
    off: ["Off", "Désactivé"], on: ["On", "Activé"], strict: ["Strict", "Strict"],
    en: ["English", "English"], fr: ["Français", "Français"],
    60: ["Every hour", "Chaque heure"], 240: ["Every 4 hours", "Toutes les 4 heures"], 720: ["Every 12 hours", "Toutes les 12 heures"],
    provider: ["By the AI provider", "Par le fournisseur d'IA"],
  };
  const worded = (written) => (WORDS[written] ? say(WORDS[written][0], WORDS[written][1]) : written);

  function row(label, about, control) {
    const line = make("div", "row");
    const words = make("div");
    words.append(make("strong", "", label));
    if (about) words.append(make("small", "", about));
    line.append(words, control);
    return line;
  }

  function chosen(key, choices, labels) {
    const choose = make("select");
    const now = value(key);
    const all = choices.includes(now) ? choices : [now, ...choices];
    for (const choice of all) {
      const one = make("option", "", (labels && labels[choice]) || worded(choice));
      one.value = choice;
      choose.append(one);
    }
    choose.value = now;
    choose.onchange = async () => {
      choose.disabled = true;
      // A change the Agent starts again for is taken a moment later.
      let taken = false;
      for (let tried = 0; tried < 20 && !taken; tried++) {
        taken = await set(key, choose.value);
        if (!taken) await new Promise((wait) => setTimeout(wait, 800));
      }
      if (!taken) choose.value = now;
      choose.disabled = false;
    };
    return choose;
  }

  async function settings() {
    head(say("Settings", "Réglages"));
    if (!(await read())) return panel.append(make("p", "quiet", say("The settings could not be read.", "Les réglages n'ont pas pu être lus.")));
    const of = (key) => (kept.settings.find((one) => one.key === key) || { choices: [] }).choices;
    const models = kept.models || [];
    const called = Object.fromEntries(models.map((model) => [model.value, model.label + (model.value === kept.usual ? say(" · Recommended", " · Recommandé") : "")]));
    // What the agent is connected to is set here too.
    const linked = make("section");
    const go = make("button", "go");
    go.type = "button";
    go.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7h4v4H7zM13 13h4v4h-4zM11 9h4a2 2 0 0 1 2 2v2" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
    const told = make("span");
    told.append(make("strong", "", say("Services", "Services")), make("small", "", say("What your agent can use for you: mail, calendar, tools.", "Ce que ton agent peut utiliser pour toi : mails, agenda, outils.")));
    go.append(told);
    go.insertAdjacentHTML("beforeend", `<svg class="next" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M9 5l7 7-7 7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`);
    go.onclick = () => open("services");
    linked.append(go);
    const thinks = make("section");
    thinks.append(make("h2", "", say("Thinks with", "Réfléchit avec")));
    if (models.length) {
      thinks.append(row(say("Model", "Modèle"), say("The recommended one makes a plan last the month.", "Le modèle recommandé fait durer un abonnement tout le mois."), chosen("model", models.map((model) => model.value), called)));
      thinks.append(row(say("Light model", "Modèle léger"), say("For summaries and wake-ups.", "Pour les résumés et les réveils."), chosen("light", ["off", ...models.map((model) => model.value)], called)));
    } else {
      thinks.append(row(say("Model", "Modèle"), "", make("span", "quiet", value("model"))));
    }
    const does = make("section");
    does.append(make("h2", "", say("How it works", "Sa façon de travailler")));
    does.append(row(say("Permission", "Autorisation"), say("How much your agent does without asking.", "Ce que ton agent fait sans demander."), chosen("permission", of("permission"))));
    does.append(row(say("Wake-ups", "Réveils"), say("How often it looks at what it has to do.", "À quelle fréquence il regarde ce qu'il a à faire."), chosen("wakeups", of("wakeups"))));
    does.append(row(say("Quiet hours", "Heures calmes"), say("When it does not write first.", "Quand il ne t'écrit pas de lui-même."), chosen("quiet", of("quiet"))));
    does.append(row(say("Passwords in messages", "Mots de passe dans les messages"), say("Whether a message that holds one reaches the AI.", "Si un message qui en contient un parvient à l'IA."), chosen("secrets", of("secrets"))));
    const rest = make("section");
    rest.append(make("h2", "", "Orbite"));
    rest.append(row(say("Language", "Langue"), say("Of Orbite's own words.", "Des textes d'Orbite."), chosen("language", of("language"))));
    rest.append(row(say("Voice messages", "Messages vocaux"), say("What writes down what you say.", "Ce qui transcrit ce que tu dis."), chosen("voice", of("voice"))));
    rest.append(row(say("Automatic updates", "Mises à jour automatiques"), "", chosen("updates", of("updates"))));
    panel.append(linked, thinks, does, rest);
  }

  async function services() {
    head(say("Services", "Services"), say("What your agent can use for you, once you have signed in.", "Ce que ton agent peut utiliser pour toi, une fois que tu t'es connecté."), ["settings", say("Settings", "Réglages")]);
    const told = await fetch("/api/connectors").catch(() => null);
    const all = told && told.ok ? await told.json() : null;
    if (!all) return panel.append(make("p", "quiet", say("The services could not be read.", "Les services n'ont pas pu être lus.")));
    if (!all.with_a_touch) panel.append(make("p", "quiet", say("Your agent's page is not reached encrypted yet: ask it in words to connect a service.", "La page de ton agent n'est pas encore joignable en chiffré : demande-lui avec tes mots de connecter un service.")));
    const list = make("section", "grid");
    for (const one of all.connectors) {
      const card = make("div", "card");
      const mark = make("span", "letter", MARKS[one.name] ? "" : one.label.slice(0, 1));
      if (MARKS[one.name]) {
        mark.classList.add("mark");
        mark.innerHTML = `<svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="${MARKS[one.name][0]}" fill="${MARKS[one.name][1]}"/></svg>`;
      }
      card.append(mark);
      const words = make("div");
      words.append(make("strong", "", one.label), make("small", "", one.does));
      card.append(words);
      if (one.connected) {
        card.append(make("span", "done", say("Connected", "Connecté")));
      } else {
        const go = make("button", "", say("Connect", "Connecter"));
        go.type = "button";
        go.disabled = !all.with_a_touch;
        go.onclick = async () => {
          go.disabled = true;
          const begun = await post("/api/connectors", { name: one.name }).catch(() => null);
          const link = begun && begun.ok ? (await begun.json()).link : null;
          if (!link) return (go.disabled = false);
          window.open(link, "_blank", "noopener");
          // Back from the service, it says so itself.
          const until = Date.now() + 5 * 60 * 1000;
          const looking = setInterval(async () => {
            if (showing !== "services" || Date.now() > until) return clearInterval(looking);
            const now = await fetch("/api/connectors").then((answer) => answer.json()).catch(() => null);
            if (now && (now.connectors.find((other) => other.name === one.name) || {}).connected) {
              clearInterval(looking);
              services();
            }
          }, 2500);
        };
        card.append(go);
      }
      list.append(card);
    }
    panel.append(list);
  }

  // The Agent's own menu: who it is, then what is its own, each a screen.
  async function agent() {
    await who();
    if (showing !== "agent") return;
    head("", "", document.body.classList.contains("desk") ? ["talk", say("Conversation", "Conversation")] : null);
    const look = me && me.look;
    const top = make("div", "whois");
    top.append(figure(look && look.outline, look && look.colour, look && look.eyes, 96));
    top.append(make("h1", "", agentName()), make("p", "", say("My agent", "Mon agent")));
    const has = me && me.computer;
    const rows = [
      ["look", say("Customize", "Personnaliser"), say("Its name, its shape, its colour.", "Son nom, sa forme, sa couleur."),
        "M12 3l2.2 5.3 5.8.5-4.4 3.8 1.3 5.6L12 15.3 7.1 18.2l1.3-5.6L4 8.8l5.8-.5z", true],
      ["screen", say("Screen", "Écran"),
        has ? say("See what it does on its computer, or take over.", "Voir ce qu'il fait sur son ordinateur, ou prendre la main.")
          : say("It has no computer on this installation.", "Il n'a pas d'ordinateur sur cette installation."),
        "M4 5h16v11H4zM9 20h6M12 16v4", !!has],
    ];
    const list = make("section");
    for (const [place, label, about, drawn, there] of rows) {
      const go = make("button", "go");
      go.type = "button";
      go.disabled = !there;
      go.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${drawn}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
      const words = make("span");
      words.append(make("strong", "", label), make("small", "", about));
      go.append(words);
      go.insertAdjacentHTML("beforeend", `<svg class="next" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M9 5l7 7-7 7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`);
      go.onclick = () => open(place);
      list.append(go);
    }
    panel.append(top, list);
    // On a server that holds several, one can be removed: with everything
    // it kept, which is said before it is done.
    const place = window.orbite || {};
    if (place.agents && place.agents.length > 1) {
      const gone = make("section");
      const remove = make("button", "go danger");
      remove.type = "button";
      remove.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
      const words = make("span");
      words.append(make("strong", "", say("Remove this agent", "Supprimer cet agent")), make("small", "", say("With its conversation, its memory and its computer.", "Avec sa conversation, sa mémoire et son ordinateur.")));
      remove.append(words);
      remove.onclick = async () => {
        const sure = say("Remove {name} and everything it kept? This cannot be undone.", "Supprimer {name} et tout ce qu'il a gardé ? C'est définitif.").replace("{name}", agentName());
        if (!window.confirm(sure)) return;
        remove.disabled = true;
        const done = await place.plain("/api/agents/" + place.chosen, { method: "DELETE", headers: { "x-orbite": "1" } }).catch(() => null);
        if (done && done.ok) return go("");
        remove.disabled = false;
      };
      gone.append(remove);
      panel.append(gone);
    }
  }

  // The Agent's screen as it is this moment, and the Owner's hand on it.
  // What taking it gave this page, while it has it.
  let taken = "";
  const hand = (body) =>
    fetch("/api/screen/hand", { method: "POST", keepalive: true, headers: { "content-type": "application/json", "x-orbite": "1" }, body: JSON.stringify({ holder: taken, ...body }) });
  function give() {
    if (!taken) return;
    const holder = taken;
    taken = "";
    hand({ do: "give", holder }).catch(() => null);
  }
  addEventListener("pagehide", give);

  async function screen() {
    head(say("Screen", "Écran"), say("{name}'s own computer, as it is right now.", "L'ordinateur de {name}, tel qu'il est en ce moment.").replace("{name}", agentName()), ["agent", agentName()]);
    const stage = make("div", "stage off");
    const shown = make("img");
    shown.alt = "";
    shown.draggable = false;
    // What is typed is caught by a field nobody sees: a picture takes no keys.
    const keys = make("textarea");
    keys.setAttribute("aria-label", say("Keyboard of the agent's computer", "Clavier de l'ordinateur de l'agent"));
    const dark = make("p", "dark", say("Its computer is off: it starts when your agent needs it, or when you take over.", "Son ordinateur est éteint : il démarre quand ton agent en a besoin, ou quand tu prends la main."));
    stage.append(shown, dark, keys);
    const bar = make("div", "hand");
    const told = make("p");
    const take = make("button");
    take.type = "button";
    bar.append(told, take);
    const typed = make("form", "typed");
    const field = make("input");
    field.placeholder = say("Type on its screen…", "Écrire sur son écran…");
    // What is written there may be a password: nothing keeps or learns it.
    for (const typing of [field, keys]) {
      typing.autocapitalize = "off";
      typing.autocomplete = "off";
      typing.spellcheck = false;
      typing.setAttribute("autocorrect", "off");
    }
    const enter = make("button", "", say("Enter", "Entrée"));
    enter.type = "button";
    typed.append(field, enter);
    panel.append(stage, bar, typed);
    const here = () => showing === "screen" && panel.contains(stage);

    function draw() {
      stage.classList.toggle("taken", taken);
      take.className = taken ? "back-to" : "";
      take.textContent = taken ? say("Give it back to {name}", "Rendre la main à {name}").replace("{name}", agentName()) : say("Take over", "Prendre le contrôle");
      told.textContent = taken
        ? say("You have its mouse and keyboard: click on the screen and type. {name} waits.", "Tu as sa souris et son clavier : clique sur l'écran et tape. {name} attend.").replace("{name}", agentName())
        : say("You see what {name} sees. Take over to do something yourself, such as signing in.", "Tu vois ce que voit {name}. Prends la main pour faire quelque chose toi-même, comme te connecter à un compte.").replace("{name}", agentName());
      // A phone has no keys until a field asks for them.
      typed.classList.toggle("shown", taken && matchMedia("(pointer: coarse)").matches);
    }

    // One gesture after another, in the order they were made.
    let line = Promise.resolve();
    const act = (body) => {
      line = line.then(async () => {
        const done = await hand(body).catch(() => null);
        if (done && done.status === 409 && taken) {
          taken = "";
          draw();
        }
        see();
      });
      return line;
    };

    let looking = false;
    async function see() {
      if (!here() || looking || document.hidden) return;
      looking = true;
      const got = await fetch("/api/screen/now" + (taken ? "?held=" + taken : ""), { cache: "no-store" }).catch(() => null);
      looking = false;
      if (!here()) return;
      if (got && got.status === 200) {
        const old = shown.src;
        shown.src = URL.createObjectURL(await got.blob());
        if (old.startsWith("blob:")) shown.addEventListener("load", () => URL.revokeObjectURL(old), { once: true });
        stage.classList.remove("off");
      } else if (got && got.status === 204) {
        stage.classList.add("off");
      }
    }
    (async function follow() {
      if (!here()) return;
      await see();
      setTimeout(follow, taken ? 450 : 1300);
    })();
    // Kept while the window is behind another, where nothing is drawn: the
    // time to fetch a code somewhere else.
    (async function hold() {
      if (!here()) return;
      if (taken && document.hidden) {
        const still = await hand({ do: "here" }).catch(() => null);
        if (still && still.status === 409) {
          taken = "";
          draw();
        }
      }
      setTimeout(hold, 15000);
    })();

    take.onclick = async () => {
      if (taken) {
        // What was typed and not sent yet goes first.
        flush();
        take.disabled = true;
        await line;
        take.disabled = false;
        give();
        return draw();
      }
      take.disabled = true;
      take.textContent = say("Starting…", "Démarrage…");
      const done = await hand({ do: "take" }).catch(() => null);
      take.disabled = false;
      taken = (done && done.ok && (await done.json()).holder) || "";
      // Left meanwhile: it is given back at once.
      if (!here()) return give();
      draw();
      if (taken) keys.focus({ preventScroll: true });
      see();
    };

    // Where on its screen a point of the picture is: the picture is shown
    // whole inside its frame, with bands beside it when the frame is wider.
    const at = (event) => {
      if (!shown.naturalWidth) return null;
      const box = shown.getBoundingClientRect();
      const scale = Math.min(box.width / shown.naturalWidth, box.height / shown.naturalHeight) || 1;
      const left = box.left + (box.width - shown.naturalWidth * scale) / 2;
      const top = box.top + (box.height - shown.naturalHeight * scale) / 2;
      const x = Math.round((event.clientX - left) / scale), y = Math.round((event.clientY - top) / scale);
      return x < 0 || y < 0 || x >= shown.naturalWidth || y >= shown.naturalHeight ? null : { x, y };
    };
    shown.addEventListener("click", (event) => {
      if (!taken) return;
      const point = at(event);
      // Letters still waiting are typed where they were meant, before the click.
      flush();
      if (point) act({ do: "click", ...point });
      if (!matchMedia("(pointer: coarse)").matches) keys.focus({ preventScroll: true });
    });
    let turned = 0, turning = null, over = null;
    stage.addEventListener("wheel", (event) => {
      if (!taken) return;
      event.preventDefault();
      turned += event.deltaY;
      over = at(event) || over;
      if (turning || !over) return;
      turning = setTimeout(() => {
        const notches = Math.max(-5, Math.min(5, Math.round(turned / 100) || Math.sign(turned)));
        turned = 0;
        turning = null;
        flush();
        if (notches) act({ do: "scroll", ...over, notches });
      }, 120);
    }, { passive: false });

    // Letters are sent together, a moment after the last one; a key that
    // is no letter is sent by its name.
    const NAMED = { Enter: "Return", Backspace: "BackSpace", Tab: "Tab", Escape: "Escape", Delete: "Delete", ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", Home: "Home", End: "End", PageUp: "Page_Up", PageDown: "Page_Down", " ": "space" };
    let letters = "", sending = null;
    // A long text goes in pieces, none of it dropped.
    const type = (text) => {
      for (let from = 0; from < text.length; from += 1500) act({ do: "type", text: text.slice(from, from + 1500) });
    };
    function flush() {
      clearTimeout(sending);
      sending = null;
      if (!letters) return;
      const text = letters;
      letters = "";
      type(text);
    }
    keys.addEventListener("keydown", (event) => {
      if (!taken || event.isComposing || event.keyCode === 229) return;
      // A letter is one however it was made: with the keys that reach a
      // keyboard's third signs too, which a French one needs for @ and #.
      const held = (event.ctrlKey || event.metaKey) && !event.getModifierState("AltGraph");
      // Pasting is left to the browser, which hands over what is pasted.
      if (held && event.key.toLowerCase() === "v") return;
      if (event.key === "Dead" || event.key === "Process") return;
      if (event.key.length === 1 && !held) {
        event.preventDefault();
        letters += event.key;
        clearTimeout(sending);
        sending = setTimeout(flush, 140);
        return;
      }
      const name = NAMED[event.key] || (event.key.length === 1 && /[a-z0-9]/i.test(event.key) ? event.key.toLowerCase() : null);
      if (!name) return;
      event.preventDefault();
      flush();
      const with_ = [(event.ctrlKey || event.metaKey) && "ctrl", event.altKey && "alt", event.shiftKey && (name.length > 1 || held) && "shift"].filter(Boolean);
      act({ do: "key", keys: [...with_, name].join("+") });
    });
    // What is pasted goes straight to its computer: no model reads it.
    keys.addEventListener("paste", (event) => {
      event.preventDefault();
      const text = (event.clipboardData || window.clipboardData).getData("text");
      flush();
      if (taken && text) type(text);
    });
    // What a keyboard composes, an accent then its letter, or a phone's
    // own way of writing, arrives as text: sent as it is.
    const composed = () => {
      const text = keys.value;
      keys.value = "";
      if (taken && text) { flush(); type(text); }
    };
    keys.addEventListener("input", (event) => { if (!event.isComposing) composed(); });
    keys.addEventListener("compositionend", composed);
    const write = () => {
      if (taken && field.value) type(field.value);
      field.value = "";
    };
    typed.onsubmit = (event) => { event.preventDefault(); write(); };
    enter.onclick = () => { write(); if (taken) act({ do: "key", keys: "Return" }); };
    draw();
  }

  async function look() {
    head(say("Customize", "Personnaliser"), say("Its name and its look, which are its own.", "Son nom et son apparence, qui lui sont propres."), ["agent", agentName()]);
    if (!(await read())) return;
    const shapes = kept.shapes || {}, colours = kept.colours || {};
    const colour = () => colours[value("colour")] || "#f2f2f0";
    const shown = make("div", "shown");
    const draw = () => {
      const shape = shapes[value("shape")];
      shown.replaceChildren(figure(shape && shape.outline, colour(), shape && shape.eyes, 132));
    };
    const change = async (key, to) => {
      const one = kept.settings.find((setting) => setting.key === key);
      if (one) one.value = to;
      draw();
      pick();
      await set(key, to);
      const again = await fetch("/api/me").catch(() => null);
      if (again && again.ok) me = await again.json();
      column();
    };
    const name = make("input");
    name.value = value("name");
    name.maxLength = 40;
    name.onchange = () => name.value.trim() && change("name", name.value.trim());
    const forms = make("div", "swatches"), tints = make("div", "swatches");
    const pick = () => {
      forms.replaceChildren();
      for (const shape of Object.keys(shapes).sort()) {
        const one = make("button", value("shape") === shape ? "here" : "");
        one.type = "button";
        one.append(figure(shapes[shape].outline, colour(), shapes[shape].eyes, 46));
        one.onclick = () => change("shape", shape);
        forms.append(one);
      }
      tints.replaceChildren();
      for (const tint of Object.keys(colours).sort()) {
        const one = make("button", "tint" + (value("colour") === tint ? " here" : ""));
        one.type = "button";
        one.style.setProperty("--tint", colours[tint]);
        one.onclick = () => change("colour", tint);
        tints.append(one);
      }
    };
    draw();
    pick();
    const named = make("section");
    named.append(row(say("Name", "Nom"), "", name));
    const formed = make("section");
    formed.append(make("h2", "", say("Shape", "Forme")), forms);
    const tinted = make("section");
    tinted.append(make("h2", "", say("Colour", "Couleur")), tints);
    panel.append(shown, named, formed, tinted);
  }

  async function arrange() {
    const here = wide.matches && !talk.hidden;
    document.body.classList.toggle("desk", here);
    side.hidden = !here;
    if (!here) {
      if (showing !== "talk") open("talk");
      panel.hidden = true;
      talk.classList.remove("aside");
      return;
    }
    if (!me) await who();
    column();
    meet();
    latest();
  }
  // The Conversation is read from its end: taken there when the column
  // comes, which changes how wide what was said is, and when it is come
  // back to from another screen.
  function latest() {
    const said = $("said");
    const go = () => { said.scrollTop = said.scrollHeight; };
    go();
    requestAnimationFrame(go);
    setTimeout(go, 250);
    setTimeout(go, 900);
  }
  // On a phone the Agent itself, at the top of the Conversation, is the way
  // to its menu.
  $("who").addEventListener("click", () => { if (!talk.hidden) open("agent"); });
  wide.addEventListener("change", arrange);
  // The Conversation shows once the device is let in: the column comes with it.
  new MutationObserver(arrange).observe(talk, { attributes: true, attributeFilter: ["hidden"] });
  arrange();
})();

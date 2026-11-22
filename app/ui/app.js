// The app's own page: it asks where the Owner's agent is, once, then shows
// the agent's page in its place every time the app opens.
(() => {
  const WORDS = {
    en: {
      title: "Open your agent",
      how: "Ask your agent to add this device, and paste the link it gives you. You do this once.",
      label: "Link from your agent", open: "Open",
      looking: "Reaching your agent…",
      away: "Your agent does not answer",
      awayWhy: "Check that this device is on the same private network as your agent's machine (Tailscale, for instance), and that the machine is on.",
      again: "Try again", other: "Use another link",
      wrong: { "not an address": "That is not a link from your agent.", "not encrypted": "That address is not encrypted: ask your agent for its link." },
    },
    fr: {
      title: "Ouvre ton agent",
      how: "Demande à ton agent d'ajouter cet appareil, et colle le lien qu'il te donne. C'est à faire une seule fois.",
      label: "Lien donné par ton agent", open: "Ouvrir",
      looking: "Connexion à ton agent…",
      away: "Ton agent ne répond pas",
      awayWhy: "Vérifie que cet appareil est sur le même réseau privé que la machine de ton agent (Tailscale, par exemple), et que la machine est allumée.",
      again: "Réessayer", other: "Utiliser un autre lien",
      wrong: { "not an address": "Ce n'est pas un lien de ton agent.", "not encrypted": "Cette adresse n'est pas chiffrée : demande son lien à ton agent." },
    },
  };
  const french = (navigator.language || "").toLowerCase().startsWith("fr");
  const words = french ? WORDS.fr : WORDS.en;
  document.documentElement.lang = french ? "fr" : "en";
  for (const said of document.querySelectorAll("[data-say]")) said.textContent = words[said.dataset.say];

  const $ = (id) => document.getElementById(id);
  const ask = window.__TAURI__.core.invoke;
  const show = (shown) => { for (const part of ["looking", "away", "link"]) $(part).hidden = part !== shown; };
  const kept = () => { try { return localStorage.getItem("agent"); } catch (lost) { return null; } };

  // Shows the agent's page once it answers; `with` is the code of a first visit.
  async function open(address, code) {
    show("looking");
    $("where").textContent = new URL(address).host;
    if (await ask("answers", { address })) {
      location.replace(code ? `${address}/#${code}` : address);
    } else {
      show("away");
    }
  }

  $("link").addEventListener("submit", async (sent) => {
    sent.preventDefault();
    $("wrong").hidden = true;
    try {
      const read = await ask("read", { pasted: $("pasted").value });
      localStorage.setItem("agent", read.address);
      open(read.address, read.code);
    } catch (why) {
      $("wrong").textContent = words.wrong[why] || words.wrong["not an address"];
      $("wrong").hidden = false;
    }
  });
  $("again").addEventListener("click", () => open(kept()));
  $("other").addEventListener("click", () => { show("link"); $("pasted").focus(); });

  if (kept()) open(kept());
  else { show("link"); $("pasted").focus(); }
})();

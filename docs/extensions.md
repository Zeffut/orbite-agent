# Extensions

An extension adds actions to your agent: a connector to a service you use, a small tool of your own. Extensions are short JavaScript programs. Most are written by the agent itself, when you ask for one and agree to what it wrote; the ones that come with Orbite are the Official Extensions.

## Living with extensions

- `/extensions` lists them and says which are on.
- `/extensions off NAME` and `/extensions on NAME` turn one off and on.
- `/extensions remove NAME` deletes one, with everything it kept. Official Extensions can be turned off, not removed.

An extension that does not load, never ends or takes too much memory is turned off by itself, and your agent tells you.

**Asking for one.** "Write yourself an extension that converts currencies with the Frankfurter API" is enough. Before anything is created, you are shown the whole script, the hosts it would reach, the secrets it would send them and the actions it adds, and you decide; this is asked at every permission level. The agent then tries what it wrote, and can read and repair it later, which asks you again.

## What an extension can and cannot do

An extension runs in a sandbox of its own, started afresh for every action. It has plain JavaScript and nothing else: no files, no processes, no network, no timers. Orbite lends it exactly two things:

- **a store of its own**, to keep data between actions;
- **the hosts it was granted**, by name, and nothing beyond them. A redirection to another host is refused.

It is stopped after 20 seconds or 32 MB. What it keeps is limited too: 1 MB under one key, 10 MB in all; and it is given at most the first 200,000 characters of a service's answer.

## Writing one

An extension is a folder in `/var/lib/orbite/extensions/`, named in lowercase words joined by dashes (plain letters and digits only), with two files. Writing there with the agent's ordinary file actions is asked about at every permission level: the agent is meant to go through `create_extension`, which shows you the whole of it.

`extension.json`:

```json
{
  "description": "Converts an amount from one currency to another.",
  "hosts": ["api.frankfurter.dev"],
  "secrets": []
}
```

`main.js` defines two global functions:

```js
function describe() {
  return [
    {
      name: "convert",
      description: "Converts an amount from one currency to another.",
      parameters: {
        amount: { type: "number", description: "The amount" },
        from: "The currency to convert from, such as EUR",
        to: "The currency to convert to, such as USD",
        "date?": "A day in the past, as YYYY-MM-DD",
      },
    },
  ];
}

function run(action, input) {
  const day = input.date || "latest";
  const answer = http.get(
    `https://api.frankfurter.dev/v1/${day}?amount=${input.amount}&from=${input.from}&to=${input.to}`
  );
  if (answer.status !== 200) throw new Error("The service answered " + answer.status);
  return input.amount + " " + input.from + " = " + answer.json().rates[input.to] + " " + input.to;
}
```

- **`describe()`** returns the actions the extension adds. Each has a `name` (lowercase letters, digits and `_`), a `description` written for the model (what it does and when to use it), and `parameters`: each parameter's name mapped to its description, or to a whole JSON Schema when it is not a text. A name ending in `?` may be left out. An action that changes something outside the extension's own store should say `asks: true`: your agent then asks you before each use. `critical: true` asks at every permission level.
- **`run(action, input)`** carries out the action called `action` and returns its result, a text or an object. It may be `async`. Throwing an `Error` tells the model what went wrong, and the extension stays on.

The agent sees each action as `extension_action`, with dashes in the extension's name turned to `_`: here, `currency_converter_convert` for a folder called `currency-converter`. That whole name is at most 64 characters, and two extensions cannot add an action of the same name.

### What Orbite lends

| | |
|---|---|
| `store.get(key)` | the value kept under `key`, or `null` |
| `store.set(key, value)` | keeps any JSON value |
| `store.remove(key)`, `store.keys()` | |
| `http.get(url, { headers })`, `http.delete(url, { headers })` | |
| `http.post(url, body, { headers })`, also `put` and `patch` | an object as `body` is sent as JSON |

Every `http` call returns `{ status, body, json() }`, and throws when the host was not granted or could not be reached.

### Secrets

An extension never holds a secret. It writes its name between double braces, in an address, a header or a body, and Orbite fills it in on the way to a granted host:

```js
http.get("https://api.example.com/v1/me", { headers: { Authorization: "Bearer {{EXAMPLE_KEY}}" } });
```

The secret must be listed in `extension.json` under `secrets`, and entered on the machine with `orbite secret set EXAMPLE_KEY`. If the service sends the secret back, or an error would quote it, the extension gets its name in its place. Orbite's own keys (those named `ORBITE_…`) are never lent.

## Official Extensions

| Name | What it adds |
|---|---|
| `tasks` | your task list: add a task, list what is left, mark one done, remove one |

// The task list: an Official Extension of Orbite.
// Tasks are kept in this Extension's own store, as { id, title, due, done }.

function describe() {
  return [
    {
      name: "add",
      description: "Adds a task to your Owner's task list. Use it when they ask to be reminded of something to do with no precise moment; for a precise moment, schedule a reminder instead.",
      parameters: { title: "What is to be done", "due?": "The day it is due, as YYYY-MM-DD, when one was given" },
    },
    {
      name: "list",
      description: "Lists the tasks still to be done, with their numbers.",
      parameters: { "done?": { type: "boolean", description: "True to list the tasks already done instead" } },
    },
    {
      name: "done",
      description: "Marks a task as done.",
      parameters: { id: { type: "integer", description: "The task's number, from the list" } },
    },
    {
      name: "remove",
      description: "Removes a task from the list for good, done or not.",
      parameters: { id: { type: "integer", description: "The task's number, from the list" } },
    },
  ];
}

function line(task) {
  return task.id + ". " + task.title + (task.due ? " (due " + task.due + ")" : "");
}

function run(action, input) {
  const tasks = store.get("tasks") || [];
  if (action === "add") {
    const title = String(input.title || "").trim();
    if (!title) throw new Error("A task needs a title.");
    const task = { id: (store.get("last") || 0) + 1, title, due: input.due || null, done: false };
    tasks.push(task);
    store.set("tasks", tasks);
    store.set("last", task.id);
    return "Added: " + line(task);
  }
  if (action === "list") {
    const wanted = tasks.filter(task => task.done === Boolean(input.done));
    // Tasks with a day come first, the soonest on top.
    wanted.sort((a, b) => (a.due || "9999").localeCompare(b.due || "9999") || a.id - b.id);
    if (!wanted.length) return input.done ? "No task is done yet." : "Nothing is left to do.";
    return wanted.map(line).join("\n");
  }
  const task = tasks.find(task => task.id === Number(input.id));
  if (!task) throw new Error("There is no task number " + input.id + ".");
  if (action === "done") {
    task.done = true;
    store.set("tasks", tasks);
    return "Done: " + line(task);
  }
  if (action === "remove") {
    store.set("tasks", tasks.filter(other => other !== task));
    return "Removed: " + line(task);
  }
  throw new Error("There is no action called " + action + ".");
}

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
let clips = [],
  folder = "",
  preset = "medium",
  running = false,
  scanning = false,
  batchFolder = "";
const ratios = { light: 0.75, medium: 0.55, strong: 0.35 };
const bytes = (n) =>
  n >= 1e9 ? `${(n / 1e9).toFixed(2)} GB` : `${(n / 1e6).toFixed(1)} MB`;
function say(message = "") {
  $("message").textContent = message;
}
function element(tag, cls, text) {
  const node = document.createElement(tag);
  node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}
function render() {
  $("count").textContent = clips.length;
  $("folder").textContent =
    folder || "Choose a folder, including an external SSD";
  $("folder").title = folder || "Choose an output folder";
  $("start").disabled =
    running || scanning || !clips.some((c) => c.state !== "done") || !folder;
  $("start").hidden = running;
  $("cancel").hidden = !running;
  for (const id of ["add", "clear", "choose"])
    $(id).disabled = running || scanning;
  document.querySelectorAll("input[name=preset]").forEach((el) => {
    el.disabled = running;
    el.closest("label").classList.toggle("selected", el.checked);
  });
  if (!clips.length) {
    $("clips").replaceChildren();
    const empty = element("div", "empty");
    empty.append(
      element("div", "drop-icon", "↧"),
      element("h3", "", "Drop your RAW clips here"),
      element("p", "", "or choose Add clips to get started"),
    );
    $("clips").append(empty);
  } else {
    $("clips").replaceChildren(
      ...clips.map((c) => {
        const row = element("div", "clip");
        row.append(element("div", "clip-icon", "RAW"));
        const info = element("div", "");
        const name = element("div", "clip-name", c.name);
        name.title = c.path;
        info.append(name);
        info.append(
          element(
            "div",
            "clip-meta",
            `${c.format} · ${c.width} × ${c.height} coded · ${c.fps.toFixed(2)} fps · ${(c.frames / c.fps).toFixed(1)} s`,
          ),
        );
        if (c.status)
          info.append(
            element(
              "div",
              `status${c.state === "error" ? " error" : ""}`,
              c.status,
            ),
          );
        if (c.state === "working") {
          const bar = element("progress", "");
          bar.max = 1;
          bar.value = c.fraction || 0;
          info.append(bar);
        }
        row.append(info);
        const size = element("div", "clip-size", bytes(c.bytes));
        size.append(
          element(
            "small",
            "",
            c.result
              ? `→ ${bytes(c.result.output_bytes)}`
              : `~ ${bytes(c.bytes * ratios[preset])} estimated`,
          ),
        );
        row.append(size);
        const remove = element("button", "remove", "×");
        remove.setAttribute("aria-label", `Remove ${c.name}`);
        remove.disabled = running;
        remove.onclick = () => {
          clips = clips.filter((x) => x !== c);
          render();
        };
        row.append(remove);
        return row;
      }),
    );
  }
  const done = clips.filter((c) => c.state === "done");
  const saved = done.reduce(
    (sum, c) => sum + c.bytes - c.result.output_bytes,
    0,
  );
  $("queue-total").textContent = clips.length
    ? `${clips.length} clip${clips.length === 1 ? "" : "s"} · ${bytes(clips.reduce((s, c) => s + c.bytes, 0))} original${done.length ? ` · ${bytes(saved)} saved` : ""}`
    : "Your originals stay untouched.";
}
async function addPaths(paths) {
  if (!paths.length || running || scanning) return;
  scanning = true;
  say("");
  $("summary").textContent = "Reading clip details…";
  render();
  try {
    const result = await invoke("scan", { paths });
    for (const c of result.clips)
      if (!clips.some((x) => x.path === c.path))
        clips.push({ ...c, state: "ready" });
    if (!folder && clips.length) folder = clips[0].parent;
    say(result.errors.join("\n"));
    $("summary").textContent = clips.length
      ? "Ready to compress"
      : "Ready when you are";
  } catch (e) {
    say(String(e));
  } finally {
    scanning = false;
    render();
  }
}
$("add").onclick = async () => {
  try {
    await addPaths(await invoke("choose_clips"));
  } catch (e) {
    say(String(e));
  }
};
$("choose").onclick = async () => {
  try {
    const result = await invoke("choose_folder");
    if (result) folder = result;
    render();
  } catch (e) {
    say(String(e));
  }
};
$("clear").onclick = () => {
  clips = [];
  say();
  $("summary").textContent = "Ready when you are";
  $("detail").textContent =
    "Compression and output checks happen on your computer.";
  render();
};
document.querySelectorAll("input[name=preset]").forEach(
  (el) =>
    (el.onchange = () => {
      preset = el.value;
      render();
    }),
);
$("start").onclick = async () => {
  running = true;
  batchFolder = "";
  say();
  $("cancel").disabled = false;
  $("summary").textContent = "Preparing batch…";
  $("detail").textContent = "Originals are read only.";
  clips
    .filter((c) => c.state !== "done")
    .forEach((c) => {
      c.state = "ready";
      c.status = "";
    });
  render();
  try {
    await invoke("start", {
      paths: clips.filter((c) => c.state !== "done").map((c) => c.path),
      folder,
      preset,
    });
  } catch (e) {
    running = false;
    say(String(e));
    render();
  }
};
$("cancel").onclick = async () => {
  try {
    await invoke("cancel");
    $("cancel").disabled = true;
    $("summary").textContent = "Cancelling…";
    $("detail").textContent = "Waiting for the current work to stop safely.";
  } catch (e) {
    say(String(e));
  }
};
const find = (path) => clips.find((c) => c.path === path);
async function connect() {
  await listen("tauri://drag-enter", () => $("dropzone").classList.add("drag"));
  await listen("tauri://drag-leave", () =>
    $("dropzone").classList.remove("drag"),
  );
  await listen("tauri://drag-drop", ({ payload }) => {
    $("dropzone").classList.remove("drag");
    addPaths(payload.paths);
  });
  await listen("batch-started", ({ payload }) => {
    batchFolder = payload;
    $("detail").textContent = payload;
  });
  await listen("batch-error", ({ payload }) => say(payload));
  await listen("job-started", ({ payload }) => {
    const c = find(payload);
    if (c) {
      c.state = "working";
      c.status = "Starting…";
      c.fraction = 0;
      $("summary").textContent = `Compressing ${c.name}`;
      render();
    }
  });
  await listen("job-progress", ({ payload: p }) => {
    const c = find(p.path);
    if (c) {
      c.fraction = p.fraction;
      c.status = `${Math.floor(p.fraction * 100)}% · ${p.fps.toFixed(1)} fps`;
      render();
    }
  });
  await listen("job-done", ({ payload: p }) => {
    const c = find(p.path);
    if (c) {
      c.state = "done";
      c.result = p.result;
      c.status = `Basic checks passed · ${(100 * (1 - p.result.output_bytes / c.bytes)).toFixed(1)}% smaller · ${p.result.seconds.toFixed(1)} s`;
      render();
    }
  });
  await listen("job-error", ({ payload: p }) => {
    const c = find(p.path);
    if (c) {
      c.state = p.cancelled ? "ready" : "error";
      c.status = p.cancelled ? "Cancelled" : p.error;
      render();
    }
  });
  await listen("batch-finished", () => {
    running = false;
    const done = clips.filter((c) => c.state === "done").length;
    $("summary").textContent = `${done} of ${clips.length} clips completed`;
    $("detail").textContent = batchFolder || "Choose a folder and try again.";
    render();
  });
  await listen("closing", () => {
    $("summary").textContent = "Stopping before closing…";
  });
  render();
}
connect().catch((e) => {
  say(`Could not initialize the app: ${e}`);
  $("start").disabled = true;
});

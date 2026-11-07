const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
let clips = [],
  folder = "",
  preset = "medium",
  running = false,
  scanning = false,
  batchFolder = "",
  mode = "beside",
  inputRoots = [],
  skippedInputs = [],
  excluded = new Set();
const ratios = { light: 0.75, medium: 0.55, strong: 0.35 };
const bytes = (n) =>
  n >= 1e9 ? `${(n / 1e9).toFixed(2)} GB` : `${(n / 1e6).toFixed(1)} MB`;
function comparablePath(path) {
  const slash = "\\";
  const prefix = slash + slash + "?" + slash;
  const value = (path.startsWith(prefix) ? path.slice(prefix.length) : path)
    .split(slash)
    .join("/");
  return /^[a-z]:/i.test(value) ? value.toLowerCase() : value;
}
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
    running ||
    scanning ||
    !clips.some((c) => c.state !== "done" && c.state !== "skipped") ||
    (mode === "folder" && !folder);
  $("start").hidden = running;
  $("cancel").hidden = !running;
  $("folder-options").hidden = mode !== "folder";
  $("trash").disabled = running || scanning;
  $("skip-compressed").disabled = running || scanning;
  $("scan-summary").textContent = skippedInputs.length
    ? `${skippedInputs.length} compressed file/folder entries skipped. Renamed copies may not be recognised.`
    : "Uses preset suffixes and RAW Compact output folders. Renamed copies may not be recognised.";
  $("scan-summary").title = skippedInputs
    .slice(0, 30)
    .map((x) => `${x.path}: ${x.reason}`)
    .join("\n");
  document.querySelectorAll('input[name="output-mode"]').forEach((el) => {
    el.disabled = running || scanning;
  });
  $("destination-note").textContent =
    mode === "beside"
      ? `Creates clip_${preset}.NEV beside each source, including files from different folders. Existing files are never replaced.`
      : "Creates a new batch folder and preserves clip names. Existing files are never replaced.";
  for (const id of ["add", "add-folder", "clear", "choose"])
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
      element("h3", "", "Drop RAW clips or folders here"),
      element("p", "", "or choose Add clips / Add folder to get started"),
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
        info.append(element("div", "clip-path", `Source: ${c.path}`));
        const sep = c.parent.includes("\\") ? "\\" : "/";
        const preview =
          mode === "beside"
            ? c.parent + sep + c.name.replace(/(\.[^.]+)$/, `_${preset}$1`)
            : folder
              ? folder + sep + "New batch" + sep + c.name
              : "Choose an output folder";
        info.append(
          element(
            "div",
            "clip-path output",
            `Copy: ${c.result?.output || c.plannedOutput || preview}`,
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
          excluded.add(c.path);
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
  if (running || scanning) return;
  for (const path of paths)
    if (!inputRoots.includes(path)) inputRoots.push(path);
  if (!inputRoots.length) return;
  scanning = true;
  say("");
  $("summary").textContent = "Reading clip details…";
  render();
  try {
    const previous = new Map(clips.map((c) => [c.path, c]));
    const roots = inputRoots.filter(
      (p) =>
        !clips.some((c) => c.path === p && c.result?.original === "trashed"),
    );
    const result = await invoke("scan", {
      paths: roots,
      skipCompressed: $("skip-compressed").checked,
    });
    const added = paths.map(comparablePath);
    for (const c of result.clips) {
      const key = comparablePath(c.path);
      if (added.some((p) => key === p || key.startsWith(p + "/")))
        excluded.delete(c.path);
    }
    clips = result.clips
      .filter((c) => !excluded.has(c.path))
      .map((c) => previous.get(c.path) || { ...c, state: "ready" });
    for (const c of previous.values())
      if (c.state === "done" && !clips.some((x) => x.path === c.path))
        clips.push(c);
    skippedInputs = result.skipped;
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
$("add-folder").onclick = async () => {
  try {
    const path = await invoke("choose_folder");
    if (path) await addPaths([path]);
  } catch (e) {
    say(String(e));
  }
};
$("skip-compressed").onchange = async () => {
  clips.forEach((c) => {
    if (c.state === "skipped") {
      c.state = "ready";
      c.status = "";
    }
  });
  await addPaths([]);
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
  inputRoots = [];
  skippedInputs = [];
  excluded.clear();
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
      clips.forEach((c) => {
        if (c.state === "skipped") {
          c.state = "ready";
          c.status = "";
          c.plannedOutput = "";
        }
      });
      render();
    }),
);
$("start").onclick = async () => {
  running = true;
  batchFolder = "";
  say();
  $("cancel").disabled = false;
  $("summary").textContent = "Preparing batch…";
  $("detail").textContent = $("trash").checked
    ? "Originals move to Trash only after full verification."
    : "Originals will be kept.";
  clips
    .filter((c) => c.state !== "done" && c.state !== "skipped")
    .forEach((c) => {
      c.state = "ready";
      c.status = "";
      c.plannedOutput = "";
    });
  render();
  try {
    await invoke("start", {
      paths: clips
        .filter((c) => c.state !== "done" && c.state !== "skipped")
        .map((c) => c.path),
      folder,
      preset,
      mode,
      moveToTrash: $("trash").checked,
      skipCompressed: $("skip-compressed").checked,
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
document.querySelectorAll('input[name="output-mode"]').forEach(
  (el) =>
    (el.onchange = () => {
      mode = el.value;
      clips.forEach((c) => {
        if (c.state === "skipped") {
          c.state = "ready";
          c.status = "";
        }
      });
      clips.forEach((c) => {
        c.plannedOutput = "";
      });
      render();
    }),
);
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
    batchFolder = payload.destination;
    for (const job of payload.jobs) {
      const c = find(job.input);
      if (c) c.plannedOutput = job.output;
    }
    $("detail").textContent = payload.destination;
    render();
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
      c.status = `${p.phase} · ${Math.floor(p.fraction * 100)}%${p.fps > 0 ? ` · ${p.fps.toFixed(1)} fps` : ""}`;
      render();
    }
  });
  await listen("job-done", ({ payload: p }) => {
    const c = find(p.path);
    if (c) {
      c.state = "done";
      c.result = p.result;
      const checked =
        p.result.verification === "full"
          ? "Full verification passed"
          : "Basic checks passed";
      const original =
        p.result.original === "trashed"
          ? "original in Trash"
          : p.result.original === "check_trash"
            ? "check Trash for original"
            : "original kept";
      c.status = `${checked} · ${original} · ${(100 * (1 - p.result.output_bytes / c.bytes)).toFixed(1)}% smaller${p.result.warning ? ` · ${p.result.warning}` : ""}`;
      render();
    }
  });
  await listen("job-skipped", ({ payload: p }) => {
    const c = find(p.path);
    if (c) {
      c.state = "skipped";
      c.plannedOutput = p.output;
      c.status = `Skipped · ${p.reason}`;
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
    const skipped = clips.filter((c) => c.state === "skipped").length;
    $("summary").textContent =
      `${done} compressed · ${skipped} skipped · ${clips.length} clips`;
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

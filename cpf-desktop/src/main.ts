import { invoke } from "@tauri-apps/api/core";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./style.css";

type Json = Record<string, unknown>;
type Tab = "overview" | "titles" | "storage" | "runtime" | "events" | "settings";
interface MachineSummary { id: string; name: string; platform: string; mode: string; backend: string; runtime_running: boolean }
interface Readiness { ready_to_attempt_boot: boolean; missing: string[]; menu_boot_verified: boolean; nand_association: string; backend: string }
interface MachineSettings { virtual_sd_enabled: boolean; sound_compatibility: boolean; bios9i: string | null; bios7i: string | null; firmware: string | null }
interface MachineDetail {
  summary: MachineSummary; profile: Json; provenance: Json; observations: Json;
  source: Json; titles: Json; sd: Json; runtime_status: Json | null;
  readiness: Readiness; events: string;
}
interface LaunchResult { pid: number; backend: string; note: string }
interface VerificationCheck { label: string; source: string; expected: string; actual: string; matches: boolean }
interface VerificationReport { checks: VerificationCheck[]; all_verified: boolean; nand_association: string }
interface RestoredFile { status: string; sha256: string | null; reversal_snapshot: string | null }
interface CleanNandResult { virtual_nand: RestoredFile; runtime_nand: RestoredFile; runtime_firmware: RestoredFile }
interface CloneResult { machine_id: string; source_machine_id: string; copied_files: number }
interface SaveImportResult { category: string; path: string; size_bytes: number; sha256: string; previous_snapshot: string | null; format_validation: string }
interface StorageComparison {
  historical_source: string; current_source: string; current_contents_indexed: boolean;
  content_equality_verified: boolean; historical_entry_count: number; current_entry_count: number | null;
  same_path_type_size: number | null; missing_paths: string[]; extra_paths: string[];
  different_type_or_size: string[];
}
interface SdSnapshot { snapshot_id: string; created_at: string; image_sha256: string; host_entry_count: number | null; manifest_sha256: string }
interface SdSnapshotSummary { snapshot_id: string; created_at: string; schema: string }
interface SdRestoreResult { restored_snapshot: string; previous_snapshot: string | null; image_sha256: string }
interface SdImportResult { staged_entries: number; staged_bytes: number; prior_snapshot: string; applies_at_next_boot: boolean }
interface ScreenEvent { kind: string; width?: number; height?: number; pitch?: number; bgra?: string;
  audio?: string; sample_rate?: number; message?: string; stage?: string; paused?: boolean; frames?: number;
  live_cartridge?: boolean; live_supported?: boolean; inserted?: boolean; title?: string | null }
interface DesktopMode { dsi: boolean; public?: boolean; machine_id: string | null }
interface ButtonPosition { left: number; top: number }
type ButtonPositions = Record<string, ButtonPosition>;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
let machines: MachineSummary[] = [];
let desktopMode: DesktopMode = {dsi: false, machine_id: null};
let current: MachineDetail | null = null;
let currentSettings: MachineSettings | null = null;
let activeTab: Tab = "overview";
let verification: VerificationReport | null = null;
let storageComparison: StorageComparison | null = null;
let latestSnapshot: SdSnapshot | null = null;
let sdSnapshots: SdSnapshotSummary[] = [];
let cartridgeSlot: Json = { inserted: false };
let consoleActive = false;
let consoleReady = false;
let consolePaused = false;
let consoleSampleRate = 32768;
let consoleProgress = "Starting DSi…";
let audioContext: AudioContext | null = null;
let audioGain: GainNode | null = null;
let nextAudioTime = 0;
let latestScreenFrame: ScreenEvent | null = null;
let screenFrameScheduled = false;
const frameCanvas = document.createElement("canvas");
let frameImageData: ImageData | null = null;
const keyboardPressed = new Set<string>();
const gamepadPressed = new Set<string>();
const sentPressed = new Set<string>();
const consoleButtons = ["UP", "DOWN", "LEFT", "RIGHT", "A", "B", "X", "Y", "L", "R", "START", "SELECT"];
let keys: Record<string, string> = {
  ArrowUp: "UP", ArrowDown: "DOWN", ArrowLeft: "LEFT", ArrowRight: "RIGHT",
  KeyZ: "B", KeyX: "A", KeyA: "Y", KeyS: "X", KeyQ: "L", KeyW: "R",
  Enter: "START", ShiftRight: "SELECT",
};
const buttonSpecs: [string, number, number, number, number][] = [
  ["UP", 151, 772, 69, 70], ["DOWN", 151, 872, 69, 70],
  ["LEFT", 100, 820, 69, 70], ["RIGHT", 201, 820, 69, 70],
  ["X", 1039, 727, 72, 73], ["Y", 970, 793, 72, 73],
  ["A", 1107, 793, 72, 73], ["B", 1039, 862, 72, 73],
  ["START", 966, 1017, 62, 63], ["SELECT", 966, 1083, 62, 63],
];
const publicButtonPositions: ButtonPositions = {
  UP: {left: 10.6, top: 64.1}, DOWN: {left: 10.6, top: 72.6},
  LEFT: {left: 6.3, top: 68.4}, RIGHT: {left: 14.9, top: 68.4},
  X: {left: 86.6, top: 62.8}, Y: {left: 80.7, top: 67.0},
  A: {left: 92.5, top: 67.0}, B: {left: 86.6, top: 71.3},
  START: {left: 82.1, top: 81.6}, SELECT: {left: 82.1, top: 86.2},
  POWER: {left: 14.9, top: 85.9},
};
const buttonElements = new Map<string, HTMLButtonElement>();
let committedButtonPositions: ButtonPositions = {};
let editingButtonPositions: ButtonPositions = {};
let layoutEditing = false;

function defaultButtonPositions(): ButtonPositions {
  if (desktopMode.public) return cloneButtonPositions(publicButtonPositions);
  const positions = Object.fromEntries(buttonSpecs.map(([name, x, y]) => [name, {left: x / 1254 * 100, top: y / 1254 * 100}]));
  positions.POWER = {left: 17.7, top: 86.1};
  return positions;
}

function cloneButtonPositions(positions: ButtonPositions): ButtonPositions {
  return Object.fromEntries(Object.entries(positions).map(([name, point]) => [name, {...point}]));
}

function applyButtonPositions(positions: ButtonPositions): void {
  for (const [name, button] of buttonElements) {
    const point = positions[name]; if (!point) continue;
    button.style.left = `${point.left}%`;
    button.style.top = `${point.top}%`;
  }
}

async function loadButtonLayout(): Promise<void> {
  const defaults = defaultButtonPositions();
  try {
    const saved = await invoke<{version: number; positions: ButtonPositions} | null>("get_button_layout");
    committedButtonPositions = cloneButtonPositions(saved?.positions || defaults);
    $("button-layout-message").textContent = saved ? "Saved button layout loaded" : "Using default button positions";
  } catch (error) {
    committedButtonPositions = defaults;
    $("button-layout-message").textContent = `Could not load saved layout: ${String(error)}`;
  }
  applyButtonPositions(committedButtonPositions);
}

function closeButtonCalibration(): void {
  layoutEditing = false;
  $("dsi-shell").classList.remove("layout-editing");
  $("button-layout-tools").hidden = true;
}

function cancelButtonCalibration(): void {
  applyButtonPositions(committedButtonPositions);
  closeButtonCalibration();
}

function beginButtonCalibration(): void {
  if (layoutEditing) return;
  if (desktopMode.public) $("dsi-settings").hidden = true;
  keyboardPressed.clear(); gamepadPressed.clear(); syncButtons();
  editingButtonPositions = cloneButtonPositions(committedButtonPositions);
  layoutEditing = true;
  $("dsi-shell").classList.add("layout-editing");
  $("button-layout-tools").hidden = false;
  $("button-layout-message").textContent = "Drag each button marker over its image. Save when aligned.";
}

async function saveButtonCalibration(): Promise<void> {
  const save = $<HTMLButtonElement>("button-layout-save"); save.disabled = true;
  try {
    await invoke("save_button_layout", {layout: {version: 1, positions: editingButtonPositions}});
    committedButtonPositions = cloneButtonPositions(editingButtonPositions);
    closeButtonCalibration();
  } catch (error) { $("button-layout-message").textContent = `Could not save: ${String(error)}`; }
  finally { save.disabled = false; }
}

function text(value: unknown): string {
  if (value === null || value === undefined || value === "") return "Unknown";
  if (typeof value === "boolean") return value ? "Yes" : "No";
  return String(value);
}
function property(object: unknown, key: string): unknown {
  return object && typeof object === "object" ? (object as Json)[key] : undefined;
}
function entry(label: string, value: unknown, mono = false): HTMLElement {
  const row = document.createElement("div"); row.className = "info-row";
  const name = document.createElement("span"); name.className = "info-label"; name.textContent = label;
  const detail = document.createElement("span"); detail.className = mono ? "info-value technical" : "info-value"; detail.textContent = text(value);
  row.append(name, detail); return row;
}
function block(title: string, rows: HTMLElement[]): HTMLElement {
  const section = document.createElement("section"); section.className = "group";
  const heading = document.createElement("h2"); heading.textContent = title;
  section.append(heading, ...rows); return section;
}
function paragraph(value: string, className = "note"): HTMLElement {
  const p = document.createElement("p"); p.className = className; p.textContent = value; return p;
}
function button(label: string, action: () => void | Promise<void>, primary = false): HTMLButtonElement {
  const element = document.createElement("button"); element.textContent = label;
  element.className = primary ? "control primary" : "control";
  element.addEventListener("click", () => { void action(); }); return element;
}
function setContent(id: string, content: HTMLElement[]): void { $(id).replaceChildren(...content); }
function notify(message: string, error = false): void {
  if (desktopMode.dsi) {
    if (error) { const status = $("dsi-screen-status"); status.textContent = message; status.classList.remove("hidden"); }
    return;
  }
  const notice = $("notice"); notice.textContent = message;
  notice.className = error ? "visible error" : "visible";
  window.setTimeout(() => { if (notice.textContent === message) notice.className = ""; }, 6000);
}
function report(error: unknown): void { notify(String(error), true); }

function renderOverview(detail: MachineDetail): void {
  const p = detail.profile, o = detail.observations, s = detail.source;
  const observed = property(o, "raw_system_info");
  const header = document.createElement("div"); header.className = "page-title";
  const title = document.createElement("h1"); title.textContent = detail.summary.name;
  const subtitle = paragraph(`${detail.summary.platform} · ${detail.summary.mode}`, "subtitle");
  header.append(title, subtitle);
  const content = document.createElement("div"); content.className = "two-column";
  content.append(
    block("Machine", [
      entry("Language", p.language), entry("Protocol", p.cpf_protocol), entry("Agent version", p.agent_version),
      entry("Device ID", p.device_id, true), entry("Capture", p.capture_id, true),
    ]),
    block("Capture observations", [
      entry("SD present", property(observed, "sd_present")),
      entry("SD capacity", `${text(o.sd_capacity_observed_bytes)} bytes`, true),
      entry("SD free", `${text(o.sd_free_observed_bytes)} bytes`, true),
      paragraph("Battery and storage readings are historical observations.")
    ]),
    block("NAND source", [
      entry("Provenance", s.label), entry("Association to Agent", p.nand_association),
      entry("SHA-256", s.calculated_sha256, true), entry("Source path", s.path, true),
      paragraph("The runtime uses generated virtual NAND and leaves this source file out of the runtime path.")
    ])
  );
  const verifyButton = button("Verify preservation files", async () => {
    if (!current) return;
    const machineId = current.summary.id;
    verifyButton.disabled = true;
    verifyButton.textContent = "Verifying…";
    try {
      const result = await invoke<VerificationReport>("verify_preservation", { machineId });
      if (current?.summary.id !== machineId) return;
      verification = result;
      notify(verification.all_verified ? "Preservation files verified" : "One or more preservation checks failed", !verification.all_verified);
      renderOverview(current);
    } catch (error) { report(error); verifyButton.disabled = false; verifyButton.textContent = "Verify preservation files"; }
  });
  const verifyActions = document.createElement("div"); verifyActions.className = "actions"; verifyActions.append(verifyButton);
  const verificationRows: HTMLElement[] = [
    paragraph("Compare the current source NAND, generated virtual NAND, and bundled capture records with their recorded SHA-256 hashes."),
    verifyActions,
  ];
  if (verification) {
    verificationRows.push(entry("Result", verification.all_verified ? "All checked files match" : "Check failed"));
    verificationRows.push(entry("NAND association", verification.nand_association));
    for (const check of verification.checks) {
      verificationRows.push(entry(`${check.label} (${check.source})`, check.matches ? "Verified" : `Mismatch: ${check.actual}`));
    }
  }
  const restore = button("Restore clean NAND", async () => {
    if (!current) return;
    const machineId = current.summary.id;
    restore.disabled = true; restore.textContent = "Restoring…";
    try {
      const result = await invoke<CleanNandResult>("restore_clean_nand", { machineId });
      if (current?.summary.id !== machineId) return;
      notify(`Clean NAND restore: ${result.virtual_nand.status}; runtime: ${result.runtime_nand.status}`);
      await selectMachine(machineId);
    } catch (error) { report(error); restore.disabled = false; restore.textContent = "Restore clean NAND"; }
  });
  restore.disabled = Boolean(detail.runtime_status?.running);
  const restoreActions = document.createElement("div"); restoreActions.className = "actions"; restoreActions.append(restore);
  const cloneName = document.createElement("input"); cloneName.type = "text"; cloneName.placeholder = "New machine name"; cloneName.maxLength = 64;
  cloneName.setAttribute("aria-label", "New machine name");
  const clone = button("Clone machine", async () => {
    if (!current) return;
    const sourceId = current.summary.id;
    clone.disabled = true; clone.textContent = "Cloning…";
    try {
      const result = await invoke<CloneResult>("clone_machine", { machineId: sourceId, newName: cloneName.value.trim() });
      machines = await invoke<MachineSummary[]>("list_machines");
      const select = $<HTMLSelectElement>("machine-select");
      select.replaceChildren(...machines.map(machine => {
        const option = document.createElement("option"); option.value = machine.id; option.textContent = machine.name; return option;
      }));
      notify(`Cloned ${result.copied_files} machine files`);
      await selectMachine(result.machine_id);
    } catch (error) { report(error); clone.disabled = false; clone.textContent = "Clone machine"; }
  });
  clone.disabled = Boolean(detail.runtime_status?.running);
  const cloneActions = document.createElement("div"); cloneActions.className = "actions"; cloneActions.append(cloneName, clone);
  setContent("overview-content", [header, content, block("Preservation verification", verificationRows),
    block("Working NAND", [paragraph("Rebuild generated NAND and firmware from their verified sources. CPF saves the current working files as reversible snapshots first; the original NAND is never written."), restoreActions,
      paragraph("Clone this machine to experiment separately. The external original NAND stays referenced rather than copied."), cloneActions])]);
}

function renderTitles(detail: MachineDetail): void {
  const heading = document.createElement("h1"); heading.textContent = "NAND titles";
  const titles = Array.isArray(detail.titles.titles) ? detail.titles.titles : [];
  const summary = paragraph(`${titles.length} titles parsed from the NAND backup. Association with the live Agent remains unverified.`);
  const table = document.createElement("table");
  const header = document.createElement("thead");
  const top = document.createElement("tr");
  for (const name of ["Title ID", "Name", "Type", "Version", "Content"]) {
    const cell = document.createElement("th"); cell.textContent = name; top.append(cell);
  }
  header.append(top); table.append(header);
  const body = document.createElement("tbody");
  for (const item of titles as Json[]) {
    const row = document.createElement("tr");
    for (const value of [item.title_id, item.display_name || "Unknown", item.type,
      property(item.tmd, "version"), item.verification_status]) {
      const cell = document.createElement("td"); cell.textContent = text(value); row.append(cell);
    }
    body.append(row);
  }
  table.append(body); const scroll = document.createElement("div"); scroll.className = "table-scroll"; scroll.append(table);
  setContent("titles-content", [heading, summary, scroll]);
}

function renderStorage(detail: MachineDetail): void {
  const heading = document.createElement("h1"); heading.textContent = "Storage";
  const sd = detail.sd;
  const actions = document.createElement("div"); actions.className = "actions";
  actions.append(button("Open preservation tools", openLegacy));
  const saveCategory = document.createElement("select"); saveCategory.setAttribute("aria-label", "Save category");
  for (const category of ["cartridge", "dsiware", "system", "sd"]) {
    const option = document.createElement("option"); option.value = category; option.textContent = category; saveCategory.append(option);
  }
  const savePath = document.createElement("input"); savePath.type = "text"; savePath.placeholder = "Full path to save file";
  savePath.setAttribute("aria-label", "Save file path");
  const saveName = document.createElement("input"); saveName.type = "text"; saveName.placeholder = "Target filename (optional)";
  saveName.setAttribute("aria-label", "Target filename");
  const importSave = button("Import save", async () => {
    if (!current) return;
    const machineId = current.summary.id;
    importSave.disabled = true; importSave.textContent = "Importing…";
    try {
      const result = await invoke<SaveImportResult>("import_save", {
        machineId, category: saveCategory.value, sourcePath: savePath.value.trim(), targetName: saveName.value.trim() || null
      });
      notify(`Imported ${result.path}${result.previous_snapshot ? " (previous save snapshotted)" : ""}`);
      await selectMachine(machineId);
    } catch (error) { report(error); importSave.disabled = false; importSave.textContent = "Import save"; }
  });
  const saveControls = document.createElement("div"); saveControls.className = "actions";
  saveControls.append(saveCategory, savePath, saveName, importSave);
  const cartridgeRows: HTMLElement[] = [];
  const inserted = cartridgeSlot.inserted === true;
  const cartridgeMetadata = property(cartridgeSlot, "metadata") as Json | undefined;
  cartridgeRows.push(entry("Slot", inserted ? "Inserted" : "Empty"));
  if (inserted) {
    cartridgeRows.push(entry("Title", cartridgeMetadata?.banner_title_english ?? cartridgeMetadata?.title_from_header),
      entry("Game code", cartridgeMetadata?.game_code, true),
      entry("SHA-256", cartridgeMetadata?.sha256, true),
      paragraph("The header is parsed; the full ROM and save format have not been validated."));
    const eject = button("Eject cartridge", async () => {
      if (!current) return;
      const machineId = current.summary.id;
      eject.disabled = true;
      try {
        await invoke("eject_cartridge", { machineId });
        notify("Cartridge ejected. Restart the DSi to apply the slot change.");
        await selectMachine(machineId);
      } catch (error) { report(error); eject.disabled = false; }
    });
    eject.disabled = Boolean(detail.runtime_status?.running);
    cartridgeRows.push(eject);
  } else {
    const cartridgePath = document.createElement("input"); cartridgePath.type = "text";
    cartridgePath.placeholder = "Full path to .nds file";
    cartridgePath.setAttribute("aria-label", "Cartridge image path");
    const insert = button("Insert cartridge", async () => {
      if (!current) return;
      const machineId = current.summary.id;
      insert.disabled = true;
      try {
        await invoke("insert_cartridge", { machineId, sourcePath: cartridgePath.value.trim() });
        notify("Cartridge inserted. Start the DSi to load it.");
        await selectMachine(machineId);
      } catch (error) { report(error); insert.disabled = false; }
    });
    insert.disabled = Boolean(detail.runtime_status?.running);
    const controls = document.createElement("div"); controls.className = "actions";
    controls.append(cartridgePath, insert); cartridgeRows.push(controls);
    cartridgeRows.push(paragraph("Select a user-owned .nds image. CPF copies it into this machine and verifies the SHA-256 hash."));
  }
  if (detail.profile.virtual_sd_mode === "runtime_image") {
    const sdPath = document.createElement("input"); sdPath.type = "text";
    sdPath.placeholder = "Full path to SD backup folder";
    sdPath.setAttribute("aria-label", "SD backup directory");
    const importSd = button("Import SD backup", async () => {
      if (!current) return;
      const machineId = current.summary.id;
      importSd.disabled = true; importSd.textContent = "Importing…";
      try {
        const result = await invoke<SdImportResult>("import_runtime_sd_backup", { machineId, sourcePath: sdPath.value.trim() });
        notify(`Staged ${result.staged_entries} SD entries; import applies at next boot`);
        await selectMachine(machineId);
      } catch (error) { report(error); importSd.disabled = false; importSd.textContent = "Import SD backup"; }
    });
    importSd.disabled = Boolean(detail.runtime_status?.running);
    const sdControls = document.createElement("div"); sdControls.className = "actions";
    sdControls.append(sdPath, importSd);
    actions.append(sdControls);
    const snapshot = button("Snapshot virtual SD", async () => {
      if (!current) return;
      const machineId = current.summary.id;
      snapshot.disabled = true; snapshot.textContent = "Snapshotting…";
      try {
        const result = await invoke<SdSnapshot>("snapshot_sd_image", { machineId });
        if (current?.summary.id !== machineId) return;
        latestSnapshot = result;
        sdSnapshots = await invoke<SdSnapshotSummary[]>("list_sd_snapshots", { machineId });
        notify(`Virtual SD snapshot ${result.snapshot_id} created`);
        renderStorage(current);
      } catch (error) { report(error); snapshot.disabled = false; snapshot.textContent = "Snapshot virtual SD"; }
    });
    snapshot.disabled = Boolean(detail.runtime_status?.running);
    actions.prepend(snapshot);
  }
  const compare = button("Compare with captured SD", async () => {
    if (!current) return;
    const machineId = current.summary.id;
    compare.disabled = true; compare.textContent = "Comparing…";
    try {
      const result = await invoke<StorageComparison>("compare_storage", { machineId });
      if (current?.summary.id !== machineId) return;
      storageComparison = result; renderStorage(current);
    } catch (error) { report(error); compare.disabled = false; compare.textContent = "Compare with captured SD"; }
  });
  const comparisonRows: HTMLElement[] = [
    paragraph("The capture contains SD paths, types, and sizes. It does not contain the files or their hashes."),
    compare,
  ];
  const snapshotRows: HTMLElement[] = [];
  if (detail.profile.virtual_sd_mode === "runtime_image") {
    snapshotRows.push(entry("Available snapshots", sdSnapshots.filter(item => item.schema === "cpf.dsi.sd-image-snapshot/1").length));
    const available = sdSnapshots.filter(item => item.schema === "cpf.dsi.sd-image-snapshot/1");
    if (available.length) {
      const picker = document.createElement("select"); picker.setAttribute("aria-label", "SD snapshot to restore");
      for (const item of available) {
        const option = document.createElement("option"); option.value = item.snapshot_id;
        option.textContent = `${item.created_at} · ${item.snapshot_id}`; picker.append(option);
      }
      const restore = button("Restore selected snapshot", async () => {
        if (!current) return;
        const machineId = current.summary.id;
        restore.disabled = true; restore.textContent = "Restoring…";
        try {
          const result = await invoke<SdRestoreResult>("restore_sd_image", { machineId, snapshotId: picker.value });
          if (current?.summary.id !== machineId) return;
          notify(`Restored SD snapshot ${result.restored_snapshot}`);
          await selectMachine(machineId);
        } catch (error) { report(error); restore.disabled = false; restore.textContent = "Restore selected snapshot"; }
      });
      restore.disabled = Boolean(detail.runtime_status?.running);
      const controls = document.createElement("div"); controls.className = "actions";
      controls.append(picker, restore); snapshotRows.push(controls);
      snapshotRows.push(paragraph("Restore verifies the selected snapshot and saves the current generated SD as a new snapshot first."));
    }
  }
  if (storageComparison) {
    const result = storageComparison;
    comparisonRows.push(entry("Historical entries", result.historical_entry_count),
      entry("Current entries", result.current_entry_count),
      entry("Same path, type, and size", result.same_path_type_size),
      entry("Current source", result.current_source),
      entry("Contents indexed", result.current_contents_indexed),
      entry("Missing paths", result.missing_paths.length),
      entry("Extra paths", result.extra_paths.length),
      entry("Changed type or size", result.different_type_or_size.length));
    if (!result.current_contents_indexed) comparisonRows.push(paragraph("Current SD contents are not indexed, so path differences cannot be calculated yet."));
    if (result.missing_paths.length) comparisonRows.push(paragraph(`Missing examples: ${result.missing_paths.slice(0, 8).join(", ")}`));
    if (result.extra_paths.length) comparisonRows.push(paragraph(`Extra examples: ${result.extra_paths.slice(0, 8).join(", ")}`));
  }
  setContent("storage-content", [heading,
    block("Virtual SD", [entry("State", sd.state), entry("Logical capacity", sd.logical_size_bytes, true),
      entry("Host sync active", sd.host_sync_active), entry("Image SHA-256", sd.image_sha256, true),
      paragraph("The historical SD capture records names and sizes; the virtual SD holds actual writable files.")]),
    block("Historical SD comparison", comparisonRows),
    block("SD snapshots", snapshotRows.length ? snapshotRows : [paragraph("No generated runtime SD image is attached to this machine.")]),
    block("Save import", [paragraph("Stage a user-owned save in the selected category. CPF snapshots an existing staged save before replacement. A filename alone does not validate its format."), saveControls]),
    block("Cartridge", cartridgeRows),
    block("Preservation operations", [paragraph("Import a user-owned SD backup folder into the generated virtual SD. CPF saves a verified snapshot first and applies the import at the next DSi boot."),
      actions, ...(latestSnapshot ? [entry("Latest snapshot", latestSnapshot.snapshot_id, true),
        entry("Snapshot SHA-256", latestSnapshot.image_sha256, true)] : [])])]);
}

function renderRuntime(detail: MachineDetail): void {
  if (desktopMode.dsi) {
    const status = $("dsi-screen-status");
    if (consoleReady) status.classList.add("hidden");
    else {
      status.textContent = consoleActive ? consoleProgress :
        detail.readiness.ready_to_attempt_boot ? "Press Power to start" : `Missing: ${detail.readiness.missing.join(", ")}`;
      status.classList.remove("hidden");
    }
    return;
  }
  const heading = document.createElement("h1"); heading.textContent = desktopMode.dsi ? "Nintendo DSi" : "Runtime";
  const ready = detail.readiness, status = detail.runtime_status || {};
  const controls = document.createElement("div"); controls.className = "actions";
  const launch = button(desktopMode.dsi ? "Start DSi" : "Open DSi", desktopMode.dsi ? launchRuntime : openDsi, true);
  launch.disabled = !ready.ready_to_attempt_boot || Boolean(status.running) || consoleActive;
  controls.append(launch);
  if (desktopMode.dsi && consoleActive) {
    controls.append(button(consolePaused ? "Resume" : "Pause", () => consoleControl({kind: "pause"})),
      button("Reset", () => consoleControl({kind: "reset"})),
      button("Stop", () => consoleControl({kind: "stop"})));
  }
  const missing = ready.missing.length ? `Missing: ${ready.missing.join(", ")}` : "All required boot files are available.";
  if (!desktopMode.dsi) {
    const classic = button("Open classic console", launchClassicRuntime);
    classic.disabled = Boolean(status.running);
    controls.append(classic);
    setContent("runtime-content", [heading,
      block("DSi machine", [entry("Backend", ready.backend), entry("Running", status.running),
        entry("Ready to boot", ready.ready_to_attempt_boot), entry("DSi Menu verified", ready.menu_boot_verified),
        paragraph(missing), controls,
        paragraph("Open DSi launches the console in its own desktop window. The runtime worker runs behind that window.")])]);
    return;
  }
  const shell = document.createElement("div"); shell.className = "console-shell";
  const frames = entry("Frames", status.frames ?? 0, true);
  const frameValue = frames.querySelector(".info-value"); if (frameValue) frameValue.id = "runtime-frame-count";
  for (const [name, id] of [["Top screen", "console-top"], ["Touch screen", "console-bottom"]]) {
    const screen = document.createElement("div"); screen.className = "console-screen";
    const label = document.createElement("div"); label.className = "screen-label"; label.textContent = name;
    const canvas = document.createElement("canvas"); canvas.id = id; canvas.width = 256; canvas.height = 192;
    canvas.setAttribute("aria-label", name);
    screen.append(label, canvas); shell.append(screen);
    if (id === "console-bottom") attachTouch(canvas);
  }
  setContent("runtime-content", [heading,
    block("Console", [paragraph(consoleActive ? (consoleReady ? "Live DSi screens" : consoleProgress) : missing), controls, shell,
      frames,
      paragraph("Arrows: D-pad · Z/X: B/A · A/S: Y/X · Q/W: L/R · Enter: Start · Right Shift: Select · F5: Reset · F6: Pause")])]);
}

function registerCalibratable(button: HTMLButtonElement, name: string, shell: HTMLElement): void {
  button.dataset.button = name;
  buttonElements.set(name, button);
  let moving: {pointerId: number; offsetX: number; offsetY: number} | null = null;
  button.addEventListener("pointerdown", event => {
    if (!layoutEditing || event.button !== 0) return;
    event.preventDefault(); event.stopImmediatePropagation();
    const rect = button.getBoundingClientRect();
    moving = {pointerId: event.pointerId, offsetX: event.clientX - rect.left, offsetY: event.clientY - rect.top};
    button.setPointerCapture(event.pointerId);
  });
  button.addEventListener("pointermove", event => {
    if (!layoutEditing || moving?.pointerId !== event.pointerId) return;
    const bounds = shell.getBoundingClientRect();
    const left = Math.max(0, Math.min(100 - button.offsetWidth / bounds.width * 100,
      (event.clientX - bounds.left - moving.offsetX) / bounds.width * 100));
    const top = Math.max(0, Math.min(100 - button.offsetHeight / bounds.height * 100,
      (event.clientY - bounds.top - moving.offsetY) / bounds.height * 100));
    editingButtonPositions[name] = {left, top};
    button.style.left = `${left}%`; button.style.top = `${top}%`;
  });
  const stop = (event: PointerEvent): void => {
    if (!layoutEditing || moving?.pointerId !== event.pointerId) return;
    moving = null;
    event.preventDefault(); event.stopImmediatePropagation();
  };
  button.addEventListener("pointerup", stop);
  button.addEventListener("pointercancel", stop);
}

function setupDsiShell(): void {
  document.addEventListener("contextmenu", event => event.preventDefault());
  attachTouch($<HTMLCanvasElement>("console-bottom"));
  const appWindow = getCurrentWindow();
  type Drag = {pointerId: number; startX: number; startY: number; x: number; y: number;
    originX: number; originY: number; ready: boolean; updating: boolean; sentX: number; sentY: number};
  let drag: Drag | null = null;
  const moveWindow = async (current: Drag): Promise<void> => {
    if (!current.ready || current.updating) return;
    current.updating = true;
    try {
      while (drag === current && (current.x !== current.sentX || current.y !== current.sentY)) {
        const x = current.x, y = current.y;
        current.sentX = x; current.sentY = y;
        await appWindow.setPosition(new LogicalPosition(
          current.originX + x - current.startX,
          current.originY + y - current.startY));
      }
    } catch (error) { report(error); }
    finally { current.updating = false; }
  };
  const startDrag = async (event: PointerEvent): Promise<void> => {
    if (event.button !== 0 || drag || layoutEditing) return;
    event.preventDefault();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const current: Drag = {pointerId: event.pointerId, startX: event.screenX, startY: event.screenY,
      x: event.screenX, y: event.screenY, originX: 0, originY: 0, ready: false,
      updating: false, sentX: event.screenX, sentY: event.screenY};
    drag = current;
    try {
      const [position, scale] = await Promise.all([appWindow.outerPosition(), appWindow.scaleFactor()]);
      if (drag !== current) return;
      current.originX = position.x / scale;
      current.originY = position.y / scale;
      current.ready = true;
      void moveWindow(current);
    } catch (error) { drag = null; report(error); }
  };
  window.addEventListener("pointermove", event => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    drag.x = event.screenX; drag.y = event.screenY;
    void moveWindow(drag);
  });
  const stopDrag = (event: PointerEvent): void => {
    if (drag?.pointerId === event.pointerId) drag = null;
  };
  window.addEventListener("pointerup", stopDrag);
  window.addEventListener("pointercancel", stopDrag);
  window.addEventListener("lostpointercapture", stopDrag);
  window.addEventListener("blur", () => { drag = null; });
  $("dsi-case").addEventListener("pointerdown", startDrag);
  const shell = $("dsi-shell");
  for (const [x, y, width, height] of [
    [3, 2, 94, 4], [4, 9, 20, 30], [76, 9, 20, 30], [17, 45, 66, 7]
  ]) {
    const zone = document.createElement("div"); zone.className = "dsi-drag-zone";
    zone.style.left = `${x}%`; zone.style.top = `${y}%`;
    zone.style.width = `${width}%`; zone.style.height = `${height}%`;
    zone.addEventListener("pointerdown", startDrag);
    shell.append(zone);
  }
  for (const [name, x, y, width, height] of buttonSpecs) {
    const control = document.createElement("button");
    control.type = "button"; control.className = "dsi-hotspot"; control.setAttribute("aria-label", name);
    control.title = name;
    control.style.left = `${x / 1254 * 100}%`; control.style.top = `${y / 1254 * 100}%`;
    control.style.width = `${width / 1254 * 100}%`; control.style.height = `${height / 1254 * 100}%`;
    registerCalibratable(control, name, shell);
    control.addEventListener("pointerdown", event => {
      if (layoutEditing || event.button !== 0) return;
      event.preventDefault(); control.setPointerCapture(event.pointerId);
      keyboardPressed.add(name); syncButtons();
    });
    const release = () => { keyboardPressed.delete(name); syncButtons(); };
    control.addEventListener("pointerup", release); control.addEventListener("pointercancel", release);
    shell.append(control);
  }
  const power = document.createElement("button"); power.type = "button";
  power.className = "dsi-hotspot dsi-power"; power.setAttribute("aria-label", "Power");
  power.title = "Left click: close DSi. Right click: restart to DSi Menu.";
  registerCalibratable(power, "POWER", shell);
  power.addEventListener("click", () => {
    if (layoutEditing) return;
    if (consoleActive) void getCurrentWindow().close();
    else void launchRuntime();
  });
  power.addEventListener("contextmenu", event => {
    event.preventDefault();
    event.stopPropagation();
    if (layoutEditing) return;
    if (!consoleReady) return;
    void (async () => {
      try {
        if (consolePaused) await invoke("console_control", {command: {kind: "pause"}});
        await invoke("console_control", {command: {kind: "reset"}});
      } catch (error) { report(error); }
    })();
  });
  shell.append(power);
  $("button-layout-save").addEventListener("click", () => { void saveButtonCalibration(); });
  $("button-layout-cancel").addEventListener("click", cancelButtonCalibration);
  $("button-layout-defaults").addEventListener("click", () => {
    editingButtonPositions = defaultButtonPositions();
    applyButtonPositions(editingButtonPositions);
    $("button-layout-message").textContent = "Default positions shown. Save to keep them.";
  });
}

async function consoleControl(command: Json): Promise<void> {
  try { await invoke("console_control", { command }); }
  catch (error) { report(error); }
}

function attachTouch(canvas: HTMLCanvasElement): void {
  let activePointer: number | null = null;
  const send = (event: PointerEvent, pressed: boolean): void => {
    if (!consoleReady || layoutEditing) return;
    const rect = canvas.getBoundingClientRect();
    const x = Math.max(0, Math.min(255, Math.round((event.clientX - rect.left) / rect.width * 255)));
    const y = Math.max(0, Math.min(191, Math.round((event.clientY - rect.top) / rect.height * 191)));
    void consoleControl({kind: "touch", x, y, pressed});
  };
  canvas.addEventListener("pointerdown", event => {
    if (layoutEditing) return;
    if (event.button !== 0 || activePointer !== null) return;
    activePointer = event.pointerId;
    canvas.setPointerCapture(event.pointerId);
    send(event, true);
  });
  canvas.addEventListener("pointermove", event => {
    if (activePointer === event.pointerId && (event.buttons & 1)) send(event, true);
  });
  const release = (event: PointerEvent) => {
    if (activePointer !== event.pointerId) return;
    activePointer = null;
    send(event, false);
  };
  canvas.addEventListener("pointerup", release);
  canvas.addEventListener("pointercancel", release);
  canvas.addEventListener("lostpointercapture", release);
}

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value); const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function drawFrame(event: ScreenEvent): void {
  const width = event.width || 0, height = event.height || 0, pitch = event.pitch || 0;
  if (!event.bgra || width < 1 || width > 1024 || height < 2 || height > 768 || pitch < width * 4) return;
  const source = decodeBase64(event.bgra);
  if (source.length < pitch * height) return;
  if (!frameImageData || frameCanvas.width !== width || frameCanvas.height !== height) {
    frameCanvas.width = width; frameCanvas.height = height;
    frameImageData = new ImageData(width, height);
  }
  const pixels = frameImageData.data;
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const from = y * pitch + x * 4, to = (y * width + x) * 4;
    pixels[to] = source[from + 2]; pixels[to + 1] = source[from + 1];
    pixels[to + 2] = source[from]; pixels[to + 3] = 255;
  }
  frameCanvas.getContext("2d")?.putImageData(frameImageData, 0, 0);
  const half = Math.floor(height / 2);
  for (const [id, offset] of [["console-top", 0], ["console-bottom", half]] as const) {
    const canvas = document.getElementById(id) as HTMLCanvasElement | null;
    if (!canvas) continue;
    const context = canvas.getContext("2d");
    context?.drawImage(frameCanvas, 0, offset, width, half, 0, 0, canvas.width, canvas.height);
  }
}

function scheduleFrame(event: ScreenEvent): void {
  latestScreenFrame = event;
  if (screenFrameScheduled) return;
  screenFrameScheduled = true;
  requestAnimationFrame(() => {
    screenFrameScheduled = false;
    const frame = latestScreenFrame;
    latestScreenFrame = null;
    if (frame && consoleReady) drawFrame(frame);
  });
}

function playAudio(encoded: string): void {
  if (!encoded || !audioContext || !audioGain || consolePaused) return;
  const data = decodeBase64(encoded), frames = Math.floor(data.length / 4);
  if (!frames) return;
  const buffer = audioContext.createBuffer(2, frames, consoleSampleRate);
  const left = buffer.getChannelData(0), right = buffer.getChannelData(1);
  const samples = new DataView(data.buffer, data.byteOffset, data.byteLength);
  for (let index = 0; index < frames; index++) {
    left[index] = samples.getInt16(index * 4, true) / 32768;
    right[index] = samples.getInt16(index * 4 + 2, true) / 32768;
  }
  const source = audioContext.createBufferSource(); source.buffer = buffer; source.connect(audioGain);
  if (nextAudioTime < audioContext.currentTime || nextAudioTime > audioContext.currentTime + .25) {
    nextAudioTime = audioContext.currentTime + .02;
  }
  source.start(nextAudioTime);
  nextAudioTime += frames / consoleSampleRate;
}

function syncButtons(): void {
  if (!consoleReady) return;
  for (const name of consoleButtons) {
    const pressed = keyboardPressed.has(name) || gamepadPressed.has(name);
    if (pressed === sentPressed.has(name)) continue;
    if (pressed) sentPressed.add(name); else sentPressed.delete(name);
    void consoleControl({kind: "input", button: name, pressed});
  }
}

function pollGamepad(): void {
  if (!consoleReady || layoutEditing) return;
  const pad = navigator.getGamepads?.().find(item => item && item.connected);
  const pressed = new Set<string>();
  if (pad) {
    const mapping = ["A", "B", "X", "Y", "L", "R", "SELECT", "START"];
    mapping.forEach((name, index) => { if (pad.buttons[index]?.pressed) pressed.add(name); });
    for (const [name, index] of [["UP", 12], ["DOWN", 13], ["LEFT", 14], ["RIGHT", 15]] as const) {
      if (pad.buttons[index]?.pressed) pressed.add(name);
    }
    if ((pad.axes[0] || 0) < -.5) pressed.add("LEFT");
    if ((pad.axes[0] || 0) > .5) pressed.add("RIGHT");
    if ((pad.axes[1] || 0) < -.5) pressed.add("UP");
    if ((pad.axes[1] || 0) > .5) pressed.add("DOWN");
  }
  gamepadPressed.clear(); pressed.forEach(name => gamepadPressed.add(name)); syncButtons();
}

async function handleScreen(event: ScreenEvent): Promise<void> {
  if (event.kind === "ready") {
    consoleReady = true; consolePaused = false;
    consoleProgress = "Live DSi screens";
    if (current) current.runtime_status = {...(current.runtime_status || {}), running: true};
    consoleSampleRate = event.sample_rate || 32768;
    if (audioContext) { nextAudioTime = audioContext.currentTime; }
    if (current) renderRuntime(current);
    notify("Console connected to melonDS");
  } else if (event.kind === "frame") {
    scheduleFrame(event); if (event.audio) playAudio(event.audio);
    if (event.frames !== undefined) {
      const count = document.getElementById("runtime-frame-count"); if (count) count.textContent = String(event.frames);
    }
  } else if (event.kind === "cartridge") {
    notify(event.inserted ? `Inserted ${event.title || "cartridge"} in Slot-1` : "Slot-1 empty");
  } else if (event.kind === "cartridge_error") {
    report(event.message || "Slot-1 operation failed");
  } else if (event.kind === "progress") {
    consoleProgress = event.message || event.stage || "Starting DSi…";
    if (current && !consoleReady) renderRuntime(current);
  } else if (event.kind === "status") {
    consolePaused = Boolean(event.paused);
    if (consolePaused && audioContext) nextAudioTime = audioContext.currentTime;
    if (current) renderRuntime(current);
  } else if (event.kind === "error") {
    report(event.message || "Console worker failed");
  } else if (event.kind === "stopped") {
    if (!consoleActive && !consoleReady) return;
    consoleActive = false; consoleReady = false; consolePaused = false;
    latestScreenFrame = null;
    keyboardPressed.clear(); gamepadPressed.clear(); sentPressed.clear();
    ($<HTMLSelectElement>("machine-select")).disabled = false;
    if (audioContext) { await audioContext.close(); audioContext = null; audioGain = null; }
    if (current) { await selectMachine(current.summary.id); }
  }
}

function renderEvents(detail: MachineDetail): void {
  const heading = document.createElement("h1"); heading.textContent = "Events";
  const log = document.createElement("pre"); log.className = "event-log"; log.textContent = detail.events || "No events recorded.";
  setContent("events-content", [heading, log]);
}

function input(label: string, name: string, value: string): HTMLElement {
  const wrapper = document.createElement("label"); wrapper.className = "field";
  const span = document.createElement("span"); span.textContent = label;
  const control = document.createElement("input"); control.name = name; control.value = value; control.autocomplete = "off";
  wrapper.append(span, control); return wrapper;
}
function renderSettings(settings: MachineSettings): void {
  const heading = document.createElement("h1"); heading.textContent = "Settings";
  const form = document.createElement("form"); form.className = "settings-form";
  const checkLabel = document.createElement("label"); checkLabel.className = "check-row";
  const checkbox = document.createElement("input"); checkbox.type = "checkbox"; checkbox.name = "virtual_sd_enabled"; checkbox.checked = settings.virtual_sd_enabled;
  const caption = document.createElement("span"); caption.textContent = "Attach virtual SD card at boot";
  checkLabel.append(checkbox, caption);
  const compatibilityLabel = document.createElement("label"); compatibilityLabel.className = "check-row";
  const compatibility = document.createElement("input"); compatibility.type = "checkbox"; compatibility.name = "sound_compatibility"; compatibility.checked = settings.sound_compatibility;
  const compatibilityCaption = document.createElement("span"); compatibilityCaption.textContent = "Built-in app compatibility (PictoChat and DSi Sound)";
  compatibilityLabel.append(compatibility, compatibilityCaption);
  form.append(checkLabel, compatibilityLabel, paragraph("Optional boot-file paths override automatic discovery. Existing NAND and firmware files are read as sources; the runtime uses generated working copies."),
    input("ARM9i BIOS", "bios9i", settings.bios9i || ""),
    input("ARM7i BIOS", "bios7i", settings.bios7i || ""),
    input("DSi firmware", "firmware", settings.firmware || ""));
  const actions = document.createElement("div"); actions.className = "actions";
  const save = button("Save settings", async () => {
    if (!current) return;
    const values = new FormData(form);
    const updated: MachineSettings = {
      virtual_sd_enabled: checkbox.checked,
      sound_compatibility: compatibility.checked,
      bios9i: String(values.get("bios9i") || "").trim() || null,
      bios7i: String(values.get("bios7i") || "").trim() || null,
      firmware: String(values.get("firmware") || "").trim() || null,
    };
    try {
      save.disabled = true;
      currentSettings = await invoke<MachineSettings>("save_machine_settings", { machineId: current.summary.id, settings: updated });
      notify("Machine settings saved"); await selectMachine(current.summary.id);
    } catch (error) { report(error); save.disabled = false; }
  }, true);
  save.type = "button"; actions.append(save); form.append(actions);
  form.addEventListener("submit", event => { event.preventDefault(); save.click(); });
  setContent("settings-content", [heading, form]);
}

function render(detail: MachineDetail): void {
  $("connection-state").textContent = detail.runtime_status?.running ? "Console running" :
    detail.readiness.ready_to_attempt_boot ? "Runtime ready" : "Setup needed";
  $("footer-machine").textContent = `${detail.summary.name} · ${detail.summary.platform}`;
  renderOverview(detail); renderTitles(detail); renderStorage(detail); renderRuntime(detail); renderEvents(detail);
  if (currentSettings) renderSettings(currentSettings);
}

async function selectMachine(id: string): Promise<void> {
  try {
    const [detail, settings, snapshots, slot] = await Promise.all([
      invoke<MachineDetail>("machine_detail", { machineId: id }),
      invoke<MachineSettings>("machine_settings", { machineId: id }),
      invoke<SdSnapshotSummary[]>("list_sd_snapshots", { machineId: id }),
      invoke<Json>("cartridge_slot", { machineId: id })
    ]);
    current = detail; currentSettings = settings;
    sdSnapshots = snapshots;
    cartridgeSlot = slot;
    verification = null;
    storageComparison = null;
    latestSnapshot = null;
    ($<HTMLSelectElement>("machine-select")).value = id;
    render(detail);
  } catch (error) { report(error); }
}

async function launchRuntime(): Promise<void> {
  if (!current) return;
  try {
    consoleActive = true; consoleReady = false;
    ($<HTMLSelectElement>("machine-select")).disabled = true;
    renderRuntime(current);
    audioContext = new AudioContext();
    audioGain = audioContext.createGain(); audioGain.gain.value = .25;
    audioGain.connect(audioContext.destination);
    await audioContext.resume();
    const launched = await invoke<LaunchResult>("start_console", { machineId: current.summary.id });
    notify(`Console starting (process ${launched.pid})`);
  } catch (error) {
    consoleActive = false;
    ($<HTMLSelectElement>("machine-select")).disabled = false;
    if (audioContext) { await audioContext.close(); audioContext = null; audioGain = null; }
    if (current) renderRuntime(current);
    report(error);
  }
}

async function loadPublicShell(): Promise<void> {
  const shell = $<HTMLImageElement>("dsi-case");
  const status = $("shell-image-status");
  shell.onerror = () => {
    shell.onerror = null;
    shell.src = "/public-shell.svg";
    status.textContent = "Custom image could not be displayed; using default shell";
  };
  try {
    const image = await invoke<string | null>("public_shell_image");
    shell.src = image || "/public-shell.svg";
    status.textContent = image ? "Custom shell active" : "Default shell";
  } catch (error) {
    shell.src = "/public-shell.svg";
    status.textContent = `Default shell (${String(error)})`;
  }
}

async function loadKeyboardConfig(): Promise<void> {
  const status = $("keyboard-config-status");
  try {
    const config = await invoke<{version: number; buttons: Record<string, string>}>("public_input_config");
    const remapped = Object.fromEntries(Object.entries(config.buttons).map(([button, code]) => [code, button]));
    keyboardPressed.clear(); syncButtons();
    keys = remapped;
    status.textContent = "Keyboard bindings loaded from input.json";
  } catch (error) {
    status.textContent = `Could not load input.json; previous bindings remain active: ${String(error)}`;
  }
}

function setupPublicSettings(): void {
  void loadPublicShell();
  $("keyboard-config-reload").addEventListener("click", () => { void loadKeyboardConfig(); });
  $("button-layout-adjust").addEventListener("click", beginButtonCalibration);
  $("shell-image-choose").addEventListener("click", async () => {
    const status = $("shell-image-status");
    try {
      const path = await invoke<string | null>("public_pick_file", {folder: false});
      if (!path) return;
      status.textContent = "Importing shell image…";
      await invoke("public_set_shell", {sourcePath: path});
      await loadPublicShell();
    } catch (error) { status.textContent = String(error); }
  });
  $("shell-image-reset").addEventListener("click", async () => {
    const status = $("shell-image-status");
    try {
      await invoke("public_set_shell", {sourcePath: null});
      await loadPublicShell();
    } catch (error) { status.textContent = String(error); }
  });
  $("dsi-settings-close").addEventListener("click", () => { $("dsi-settings").hidden = true; });
  $("dsi-settings-save").addEventListener("click", async () => {
    if (!current || !currentSettings) return;
    const button = $<HTMLButtonElement>("dsi-settings-save"); button.disabled = true;
    $("dsi-settings-status").textContent = "Saving settings and restarting…";
    try {
      await invoke("stop_console");
      consoleActive = false; consoleReady = false;
      if (audioContext) { await audioContext.close(); audioContext = null; audioGain = null; }
      currentSettings = await invoke<MachineSettings>("save_machine_settings", {machineId: current.summary.id, settings: {
        ...currentSettings,
        virtual_sd_enabled: $<HTMLInputElement>("dsi-setting-sd").checked,
      }});
      $("dsi-settings").hidden = true;
      await launchRuntime();
    } catch (error) { $("dsi-settings-status").textContent = String(error); }
    finally { button.disabled = false; }
  });
}

function togglePublicSettings(): void {
  if (!desktopMode.public || !currentSettings) return;
  const panel = $("dsi-settings");
  if (panel.hidden) {
    keyboardPressed.clear(); syncButtons();
    $<HTMLInputElement>("dsi-setting-sd").checked = currentSettings.virtual_sd_enabled;
    $("dsi-settings-status").textContent = "";
  }
  panel.hidden = !panel.hidden;
}

async function openDsi(): Promise<void> {
  if (!current) return;
  try {
    const launched = await invoke<LaunchResult>("open_dsi", {machineId: current.summary.id});
    notify(`DSi opened (process ${launched.pid})`);
  } catch (error) { report(error); }
}

async function launchClassicRuntime(): Promise<void> {
  if (!current) return;
  try {
    const launched = await invoke<LaunchResult>("launch_runtime", { machineId: current.summary.id });
    notify(`Classic console starting (process ${launched.pid})`);
  } catch (error) { report(error); }
}

async function openLegacy(): Promise<void> {
  try { await invoke<LaunchResult>("open_legacy_tools"); notify("Preservation tools opened"); }
  catch (error) { report(error); }
}

async function initialize(): Promise<void> {
  try {
    desktopMode = await invoke<DesktopMode>("desktop_mode");
    ($("dsi-case") as HTMLImageElement).src = desktopMode.public ? "/public-shell.svg" : "/dsi-off.png";
    if (desktopMode.public) {
      const status = await invoke<{configured: boolean; message: string}>("public_setup_status");
      if (!status.configured) { setupPublicImport(status.message); return; }
    }
    if (desktopMode.dsi) {
      document.body.classList.add("dsi-app");
      document.documentElement.classList.add("dsi-root");
      document.title = "DSi";
      $("brand-name").textContent = "DSi";
      setupDsiShell();
      await loadButtonLayout();
      if (desktopMode.public) { setupPublicSettings(); await loadKeyboardConfig(); }
      activeTab = "runtime";
      document.querySelectorAll(".pane").forEach(item => item.classList.toggle("active", item.id === "runtime"));
    }
    await listen<ScreenEvent>("cpf-screen", event => { void handleScreen(event.payload); });
    machines = await invoke<MachineSummary[]>("list_machines");
    if (desktopMode.dsi) machines = machines.filter(machine => machine.mode === "DSi");
    const select = $<HTMLSelectElement>("machine-select");
    select.replaceChildren(...machines.map(machine => {
      const option = document.createElement("option"); option.value = machine.id; option.textContent = machine.name; return option;
    }));
    select.addEventListener("change", () => { void selectMachine(select.value); });
    if (!machines.length) { $("connection-state").textContent = "No machines found"; return; }
    const selected = machines.find(m => m.id === desktopMode.machine_id) || machines.find(m => m.name === "My DSi") || machines[0];
    await selectMachine(selected.id);
    if (desktopMode.dsi && current?.readiness.ready_to_attempt_boot && !current.runtime_status?.running) {
      await launchRuntime();
    }
  } catch (error) { report(error); $("connection-state").textContent = "Workspace unavailable"; }
}

function setupPublicImport(message: string): void {
  document.body.classList.add("public-setup-active");
  document.title = "Set up Nintendo DSi";
  const keys = [
    ["bios9", "DS ARM9 BIOS", "bios9.bin (4 KiB)"],
    ["bios7", "DS ARM7 BIOS", "bios7.bin (16 KiB)"],
    ["bios9i", "DSi ARM9i BIOS", "dsi_bios9.bin (64 KiB)"],
    ["bios7i", "DSi ARM7i BIOS", "dsi_bios7.bin (64 KiB)"],
    ["firmware", "DSi firmware", "dsi_firmware.bin"],
    ["nand", "DSi NAND", "dsi_nand.bin (240 or 256 MiB)"],
  ] as const;
  const fields = $("setup-fields");
  $("setup-status").textContent = message;
  for (const [key, label, hint] of keys) {
    const row = document.createElement("div"); row.className = "setup-row";
    const title = document.createElement("label"); title.textContent = label; title.htmlFor = `setup-${key}`;
    const input = document.createElement("input"); input.id = `setup-${key}`; input.placeholder = hint; input.autocomplete = "off";
    const browse = document.createElement("button"); browse.type = "button"; browse.textContent = "Browse…";
    browse.addEventListener("click", async () => {
      try { const path = await invoke<string | null>("public_pick_file", {folder: false}); if (path) input.value = path; }
      catch (error) { $("setup-status").textContent = String(error); }
    });
    row.append(title, input, browse); fields.append(row);
  }
  $("setup-folder").addEventListener("click", async () => {
    try {
      const folder = await invoke<string | null>("public_pick_file", {folder: true});
      if (!folder) return;
      const found = await invoke<Record<string, string>>("public_scan_folder", {folder});
      for (const [key, path] of Object.entries(found)) $<HTMLInputElement>(`setup-${key}`).value = path;
      $("setup-folder-result").textContent = `${Object.keys(found).length} of 6 files found`;
    } catch (error) { $("setup-status").textContent = String(error); }
  });
  $("setup-import").addEventListener("click", async () => {
    const button = $<HTMLButtonElement>("setup-import"); button.disabled = true;
    $("setup-status").textContent = "Validating files and creating a writable NAND copy. This may take a few minutes…";
    const files = Object.fromEntries(keys.map(([key]) => [key, $<HTMLInputElement>(`setup-${key}`).value.trim()]));
    try {
      await invoke("public_import", {files});
      $("setup-status").textContent = "Import complete. Starting DSi…";
      document.body.classList.remove("public-setup-active");
      const window = getCurrentWindow();
      await window.setDecorations(false);
      await window.setSize(new LogicalSize(628, 628));
      await window.center();
      await initialize();
    } catch (error) { $("setup-status").textContent = String(error); button.disabled = false; }
  });
}

document.querySelectorAll<HTMLButtonElement>(".tab").forEach(tab => tab.addEventListener("click", () => {
  activeTab = tab.dataset.tab as Tab;
  document.querySelectorAll(".tab").forEach(item => item.classList.toggle("active", item === tab));
  document.querySelectorAll(".pane").forEach(item => item.classList.toggle("active", item.id === activeTab));
}));
window.setInterval(async () => {
  if (!current || activeTab !== "runtime") return;
  try {
    current.runtime_status = await invoke<Json | null>("runtime_status", { machineId: current.summary.id });
    if (!consoleActive) renderRuntime(current);
    $("connection-state").textContent = consoleActive || current.runtime_status?.running ? "Console running" : "Runtime ready";
  } catch { /* A transient status read does not interrupt the current screen. */ }
}, 2000);
document.addEventListener("keydown", event => {
  if (layoutEditing) {
    if (event.code === "Escape" || event.code === "F9") { event.preventDefault(); cancelButtonCalibration(); }
    else event.preventDefault();
    return;
  }
  if (desktopMode.dsi && document.body.classList.contains("dsi-app") && event.code === "F9") { event.preventDefault(); if (!event.repeat) beginButtonCalibration(); return; }
  if (desktopMode.public && event.code === "F8") { event.preventDefault(); if (!event.repeat) togglePublicSettings(); return; }
  if (desktopMode.dsi && event.code === "Escape") { event.preventDefault(); if (desktopMode.public && !$("dsi-settings").hidden) { $("dsi-settings").hidden = true; return; } void getCurrentWindow().close(); return; }
  if (desktopMode.public && !$("dsi-settings").hidden) return;
  if (!consoleReady) return;
  if (event.code === "F5") { event.preventDefault(); if (!event.repeat) void consoleControl({kind: "reset"}); return; }
  if (event.code === "F6") { event.preventDefault(); if (!event.repeat) void consoleControl({kind: "pause"}); return; }
  const name = keys[event.code];
  if (!name || event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return;
  event.preventDefault(); keyboardPressed.add(name); syncButtons();
});
document.addEventListener("keyup", event => {
  const name = keys[event.code]; if (!name) return;
  keyboardPressed.delete(name); syncButtons();
});
window.addEventListener("blur", () => { keyboardPressed.clear(); syncButtons(); });
window.setInterval(pollGamepad, 33);
void initialize();

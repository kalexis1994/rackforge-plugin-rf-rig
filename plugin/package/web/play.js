/**
 * RF-Rig PLAY surface.
 *
 * The page builds itself from the parameter schema the host sends back, so it
 * has no private list of pedals or knobs: adding a control in the Rust contract
 * makes it appear here. The one convention it relies on is the naming inside a
 * page — `<pedal>.engaged` is the footswitch, `<pedal>.position` is where the
 * pedal sits on the board, everything else is a control.
 *
 * Three things this surface has to get right to be usable on a stage rather
 * than in a screenshot.
 *
 * **It must not rebuild itself while a hand is on it.** The host sends a fresh
 * context whenever the session revision moves — which includes every write this
 * page makes — and replacing the DOM under a finger loses the gesture. The
 * board is built once and patched thereafter.
 *
 * **It must not flood the host.** A Rack Slot edits its plugin through an
 * isolated instance: every `set_parameter` opens a plugin, loads state, applies
 * one value and saves it again. A knob dragged at sixty frames a second would
 * ask for sixty of those. Writes are coalesced per parameter and sent one at a
 * time, at most sixteen times a second.
 *
 * **A value being edited belongs to the person editing it.** While a control is
 * held, values arriving from the host for that parameter are ignored, because
 * they are echoes of writes already in flight.
 */
(function () {
  "use strict";

  const PROTOCOL = "rackforge.plugin.web@1";
  const RIG_PAGE = "rig";
  /// Milliseconds between flushes of the write queue.
  const WRITE_INTERVAL = 60;
  /// How long a status message stays before the connection line returns.
  const STATUS_LINGER = 4000;
  /**
   * How long to wait for the host before giving up on a request.
   *
   * The write queue runs one request at a time and waits for each answer, so a
   * request that is never answered is a queue that never moves again. There is
   * no reply that says "the plugin crashed"; there is only silence. Generous
   * enough that a slow slot edit is never mistaken for one.
   */
  const REQUEST_TIMEOUT = 8000;

  const pending = new Map();
  let nextRequest = 1;

  const state = {
    schema: null,
    values: new Map(),
    queue: new Map(),
    held: new Set(),
    /// For each parameter, the epoch at which this page last wrote it.
    writtenAt: new Map(),
    controls: new Map(),
    cards: new Map(),
    pedals: [],
    sounds: [],
    selectedSoundId: "",
    edited: false,
    surface: "play",
    built: false,
  };

  let writeTimer = null;
  let writing = false;
  let lastWriteAt = 0;
  /**
   * Counts every value this page has decided on. A read carries the epoch it
   * was issued at, and a value is only believed if nothing was written to that
   * parameter since — which is the only way to be sure a reply describes a
   * moment after the write rather than before it. Checking the queue is not
   * enough: between leaving the queue and the host acknowledging it, a write
   * exists nowhere the reply can see.
   */
  let writeEpoch = 0;
  let statusTimer = null;
  let refreshTimer = null;

  const boardElement = document.getElementById("board");
  const rigElement = document.getElementById("rig");
  const presetElement = document.getElementById("presets");
  const statusElement = document.getElementById("status");

  /* -------------------------------------------------------------- protocol */

  function call(method, params) {
    const requestId = "rf-rig-" + nextRequest++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(requestId);
        reject(new Error("RackForge did not answer in time"));
      }, REQUEST_TIMEOUT);
      pending.set(requestId, {
        resolve: (result) => {
          clearTimeout(timer);
          resolve(result);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      parent.postMessage(
        {
          protocol: PROTOCOL,
          kind: "request",
          request_id: requestId,
          method: method,
          params: params || {},
        },
        "*",
      );
    });
  }

  window.addEventListener("message", (event) => {
    const message = event.data;
    if (!message || message.protocol !== PROTOCOL) return;

    if (message.kind === "context") {
      state.surface = message.surface || "play";
      const instance = message.instance || {};
      state.selectedSoundId = instance.selected_sound_id || state.selectedSoundId;
      state.sounds = instance.sounds || state.sounds;
      renderPresetList();
      // A context arrives on every revision, including the ones this page
      // causes. Coalesce them, and never read back while a write is queued or
      // in flight — that read would answer with a value already superseded.
      scheduleRefresh(120);
      return;
    }

    if (message.kind !== "response") return;
    const waiting = pending.get(message.request_id);
    if (!waiting) return;
    pending.delete(message.request_id);
    if (message.ok) {
      waiting.resolve(message.result);
    } else {
      waiting.reject(new Error(message.error || "RackForge refused the request"));
    }
  });

  /* --------------------------------------------------------------- input */

  /**
   * Pointer capture keeps a drag alive when the cursor leaves the control.
   * Losing it is a nuisance, not a failure — the gesture still works while the
   * pointer stays inside — so never let it take the gesture down with it.
   */
  function capture(element, pointerId) {
    try {
      element.setPointerCapture(pointerId);
    } catch (error) {
      /* the pointer is gone, or synthetic */
    }
  }

  function releaseCapture(element, pointerId) {
    try {
      element.releasePointerCapture(pointerId);
    } catch (error) {
      /* never captured, or already released */
    }
  }

  /**
   * Every gesture in progress, so that any of them can be ended from outside.
   *
   * A gesture normally ends with a pointerup. Sometimes there isn't one: the
   * window loses focus mid-drag, the panel is hidden, the pointer is a pen that
   * left the tablet. A knob that never hears the release stays in `state.held`
   * for the rest of the session, and a parameter in `state.held` never accepts
   * another value from the host — the control would silently stop tracking. So
   * the surface takes the hint from anywhere it can get one.
   */
  const gestures = new Set();

  function endGestures() {
    [...gestures].forEach((finish) => finish());
  }

  window.addEventListener("blur", endGestures);
  window.addEventListener("pagehide", endGestures);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) endGestures();
  });

  /* ---------------------------------------------------------------- status */

  function say(text, isError) {
    statusElement.textContent = text;
    statusElement.classList.toggle("error", Boolean(isError));
    clearTimeout(statusTimer);
    statusTimer = setTimeout(() => {
      statusElement.classList.remove("error");
      statusElement.textContent = connectionLine();
    }, STATUS_LINGER);
  }

  function connectionLine() {
    const board = state.sounds.find((sound) => sound.id === state.selectedSoundId);
    const name = board ? board.name : "Custom board";
    return name + (state.edited ? " · edited" : "") + " · " + state.surface + " surface";
  }

  function idle() {
    if (statusElement.classList.contains("error")) return;
    statusElement.textContent = connectionLine();
  }

  /* ----------------------------------------------------------------- model */

  function parametersOfPage(pageId) {
    return state.schema.parameters
      .filter((parameter) => parameter.page === pageId)
      .sort((left, right) => (left.order || 0) - (right.order || 0));
  }

  function bySuffix(pageId, suffix) {
    return state.schema.parameters.find(
      (parameter) => parameter.id === pageId + "." + suffix,
    );
  }

  function collectPedals() {
    return state.schema.pages
      .filter((page) => page.id !== RIG_PAGE)
      .map((page) => ({
        page: page,
        engaged: bySuffix(page.id, "engaged"),
        position: bySuffix(page.id, "position"),
      }))
      .filter((pedal) => pedal.engaged && pedal.position);
  }

  function boardOrder() {
    return state.pedals
      .map((pedal, declaration) => ({ pedal: pedal, declaration: declaration }))
      .sort((left, right) => {
        const difference = valueOf(left.pedal.position) - valueOf(right.pedal.position);
        return difference !== 0 ? difference : left.declaration - right.declaration;
      })
      .map((entry) => entry.pedal);
  }

  /** "Fuzz Tone" rather than "Tone" — there are five of most controls. */
  function label(parameter) {
    const page = state.schema.pages.find((entry) => entry.id === parameter.page);
    if (!page || page.id === RIG_PAGE) return parameter.name;
    return page.name + " " + parameter.name;
  }

  function valueOf(parameter) {
    const stored = state.values.get(parameter.index);
    if (stored !== undefined) return stored;
    const kind = parameter.kind;
    if (kind.type === "boolean") return kind.default ? 1 : 0;
    return kind.default;
  }

  /* ------------------------------------------------------------ write path */

  function write(parameter, value) {
    if (state.values.get(parameter.index) === value) return;
    writeEpoch += 1;
    state.values.set(parameter.index, value);
    state.queue.set(parameter.index, value);
    state.writtenAt.set(parameter.index, writeEpoch);
    if (!state.edited) {
      state.edited = true;
      idle();
    }
    scheduleFlush();
  }

  function scheduleFlush() {
    if (writeTimer !== null || writing) return;
    const due = Math.max(0, WRITE_INTERVAL - (Date.now() - lastWriteAt));
    writeTimer = setTimeout(flush, due);
  }

  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  async function flush() {
    writeTimer = null;
    if (writing || state.queue.size === 0) return;
    writing = true;
    try {
      while (state.queue.size > 0) {
        // The interval has to hold *between* writes, not merely between
        // flushes. Pacing off the host's reply instead would make a fast host
        // the worst case: answer in fifteen milliseconds and the surface
        // gratefully asks sixty-six times a second. Waiting here also lets a
        // knob still in motion overwrite its own queued value, so the write
        // that eventually goes out is the newest one rather than the oldest.
        const idle = Date.now() - lastWriteAt;
        if (idle < WRITE_INTERVAL) await pause(WRITE_INTERVAL - idle);

        const [index, value] = state.queue.entries().next().value;
        state.queue.delete(index);
        lastWriteAt = Date.now();
        try {
          // One at a time: the host's slot editor opens an instance per write,
          // and asking for several at once only queues them somewhere else.
          await call("plugin.set_parameter", {
            parameter_index: index,
            value: value,
          });
        } catch (error) {
          const parameter = state.schema.parameters.find(
            (candidate) => candidate.index === index,
          );
          say((parameter ? label(parameter) : "That control") + ": " + error.message, true);
        }
      }
    } finally {
      writing = false;
      if (state.queue.size > 0) scheduleFlush();
    }
  }

  /* ------------------------------------------------------------- rendering */

  function formatValue(parameter, value) {
    const kind = parameter.kind;
    if (kind.type === "float") {
      const unit = kind.unit ? " " + kind.unit : "";
      const span = kind.maximum - kind.minimum;
      const digits = span > 100 ? 0 : span > 4 ? 1 : 2;
      return value.toFixed(digits) + unit;
    }
    if (kind.type === "enum") {
      const choice = kind.choices.find((entry) => entry.value === Math.round(value));
      return choice ? choice.name : String(value);
    }
    if (kind.type === "boolean") return value >= 0.5 ? "on" : "off";
    if (kind.type === "integer") {
      return String(Math.round(value)) + (kind.unit ? " " + kind.unit : "");
    }
    return String(value);
  }

  function knobControl(parameter) {
    const kind = parameter.kind;
    const wrapper = document.createElement("div");
    wrapper.className = "knob";

    const size = 46;
    const radius = 18;
    const centre = size / 2;
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 " + size + " " + size);
    svg.setAttribute("width", String(size));
    svg.setAttribute("height", String(size));
    svg.setAttribute("role", "slider");
    svg.setAttribute("tabindex", "0");
    svg.setAttribute("aria-label", parameter.name);
    svg.setAttribute("aria-valuemin", String(kind.minimum));
    svg.setAttribute("aria-valuemax", String(kind.maximum));

    const track = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    track.setAttribute("cx", String(centre));
    track.setAttribute("cy", String(centre));
    track.setAttribute("r", String(radius));
    track.setAttribute("class", "knob-body");
    svg.appendChild(track);

    const sweep = document.createElementNS("http://www.w3.org/2000/svg", "path");
    sweep.setAttribute("class", "knob-sweep");
    sweep.setAttribute("fill", "none");
    svg.appendChild(sweep);

    const pointer = document.createElementNS("http://www.w3.org/2000/svg", "line");
    pointer.setAttribute("class", "knob-pointer");
    svg.appendChild(pointer);

    const label = document.createElement("div");
    label.className = "label";
    label.textContent = parameter.name;
    const reading = document.createElement("div");
    reading.className = "reading";

    const normalise = (value) => (value - kind.minimum) / (kind.maximum - kind.minimum);
    const angleOf = (value) => (-225 + 270 * normalise(value)) * (Math.PI / 180);
    const pointAt = (angle, distance) => [
      centre + Math.cos(angle) * distance,
      centre + Math.sin(angle) * distance,
    ];

    function paint(value) {
      const angle = angleOf(value);
      const [tipX, tipY] = pointAt(angle, radius * 0.78);
      pointer.setAttribute("x1", String(centre));
      pointer.setAttribute("y1", String(centre));
      pointer.setAttribute("x2", String(tipX));
      pointer.setAttribute("y2", String(tipY));

      // A filled arc from the bottom-left rest position: at a glance, how far
      // up the control is without reading the number.
      const start = angleOf(kind.minimum);
      const [startX, startY] = pointAt(start, radius * 0.92);
      const [endX, endY] = pointAt(angle, radius * 0.92);
      const large = 270 * normalise(value) > 180 ? 1 : 0;
      sweep.setAttribute(
        "d",
        "M " + startX + " " + startY +
          " A " + radius * 0.92 + " " + radius * 0.92 + " 0 " + large + " 1 " + endX + " " + endY,
      );

      reading.textContent = formatValue(parameter, value);
      svg.setAttribute("aria-valuenow", String(value));
      svg.setAttribute("aria-valuetext", reading.textContent);
    }

    function quantise(value) {
      const step = kind.step || 0.001;
      const stepped = Math.round(value / step) * step;
      return Math.min(kind.maximum, Math.max(kind.minimum, stepped));
    }

    let dragging = false;
    let pointerId = null;
    let startY = 0;
    let startValue = 0;

    svg.addEventListener("pointerdown", (event) => {
      dragging = true;
      pointerId = event.pointerId;
      startY = event.clientY;
      startValue = valueOf(parameter);
      state.held.add(parameter.index);
      capture(svg, pointerId);
      svg.classList.add("held");
      gestures.add(finish);
      event.preventDefault();
    });

    svg.addEventListener("pointermove", (event) => {
      if (!dragging) return;
      // Two hundred pixels covers the range; shift slows it to a crawl for the
      // settings that need it.
      const scale = event.shiftKey ? 800 : 200;
      const delta = ((startY - event.clientY) / scale) * (kind.maximum - kind.minimum);
      const value = quantise(startValue + delta);
      paint(value);
      write(parameter, value);
    });

    function finish() {
      if (!dragging) return;
      dragging = false;
      gestures.delete(finish);
      state.held.delete(parameter.index);
      svg.classList.remove("held");
      releaseCapture(svg, pointerId);
      // The host's value is authoritative again now that nobody is holding it.
      scheduleRefresh();
    }

    svg.addEventListener("pointerup", finish);
    svg.addEventListener("pointercancel", finish);
    svg.addEventListener("lostpointercapture", finish);

    svg.addEventListener("dblclick", () => {
      paint(kind.default);
      write(parameter, kind.default);
    });

    svg.addEventListener("keydown", (event) => {
      const step = (kind.maximum - kind.minimum) / (event.shiftKey ? 200 : 20);
      let value = valueOf(parameter);
      if (event.key === "ArrowUp" || event.key === "ArrowRight") value += step;
      else if (event.key === "ArrowDown" || event.key === "ArrowLeft") value -= step;
      else if (event.key === "Home") value = kind.minimum;
      else if (event.key === "End") value = kind.maximum;
      else return;
      event.preventDefault();
      value = quantise(value);
      paint(value);
      write(parameter, value);
    });

    paint(valueOf(parameter));
    wrapper.append(svg, label, reading);
    state.controls.set(parameter.index, { apply: paint });
    return wrapper;
  }

  function choiceControl(parameter) {
    const wrapper = document.createElement("div");
    wrapper.className = "knob wide";
    const label = document.createElement("div");
    label.className = "label";
    label.textContent = parameter.name;
    const row = document.createElement("div");
    row.className = "choice";
    row.setAttribute("role", "radiogroup");
    row.setAttribute("aria-label", parameter.name);

    const buttons = parameter.kind.choices.map((choice) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = choice.name;
      button.setAttribute("role", "radio");
      button.addEventListener("click", () => {
        paint(choice.value);
        write(parameter, choice.value);
      });
      row.appendChild(button);
      return button;
    });

    function paint(value) {
      const selected = Math.round(value);
      buttons.forEach((button, index) => {
        const isSelected = parameter.kind.choices[index].value === selected;
        button.setAttribute("aria-checked", String(isSelected));
        button.setAttribute("aria-pressed", String(isSelected));
      });
    }

    paint(valueOf(parameter));
    wrapper.append(label, row);
    state.controls.set(parameter.index, { apply: paint });
    return wrapper;
  }

  function control(parameter) {
    return parameter.kind.type === "enum" ? choiceControl(parameter) : knobControl(parameter);
  }

  function cable() {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("class", "cable");
    svg.setAttribute("viewBox", "0 0 34 60");
    svg.setAttribute("aria-hidden", "true");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M0 18 C 10 18, 12 40, 17 40 C 22 40, 24 18, 34 18");
    path.setAttribute("class", "cable-line");
    path.setAttribute("fill", "none");
    svg.appendChild(path);
    return svg;
  }

  /* --------------------------------------------------------------- the board */

  function pedalCard(pedal) {
    const card = document.createElement("section");
    card.className = "pedal";
    card.dataset.page = pedal.page.id;
    card.setAttribute("aria-label", pedal.page.name);

    const head = document.createElement("div");
    head.className = "pedal-head";
    const grip = document.createElement("span");
    grip.className = "grip";
    grip.title = "Drag to move this pedal along the board";
    grip.setAttribute("aria-hidden", "true");
    grip.textContent = "⠿";
    const name = document.createElement("span");
    name.className = "pedal-name";
    name.textContent = pedal.page.name.toUpperCase();

    const moveBox = document.createElement("div");
    moveBox.className = "move";
    const earlier = document.createElement("button");
    earlier.type = "button";
    earlier.textContent = "◀";
    earlier.title = "Move earlier in the chain";
    earlier.setAttribute("aria-label", "Move " + pedal.page.name + " earlier");
    earlier.addEventListener("click", () => move(pedal, -1));
    const later = document.createElement("button");
    later.type = "button";
    later.textContent = "▶";
    later.title = "Move later in the chain";
    later.setAttribute("aria-label", "Move " + pedal.page.name + " later");
    later.addEventListener("click", () => move(pedal, 1));
    moveBox.append(earlier, later);
    head.append(grip, name, moveBox);

    const circuit = document.createElement("p");
    circuit.className = "pedal-circuit";
    circuit.textContent = pedal.page.header || "";

    const knobs = document.createElement("div");
    knobs.className = "knobs";
    parametersOfPage(pedal.page.id)
      .filter(
        (parameter) =>
          parameter.index !== pedal.engaged.index && parameter.index !== pedal.position.index,
      )
      .forEach((parameter) => knobs.appendChild(control(parameter)));

    const footer = document.createElement("div");
    footer.className = "footer";
    const lamp = document.createElement("span");
    lamp.className = "lamp";
    const stomp = document.createElement("button");
    stomp.type = "button";
    stomp.className = "stomp";
    stomp.setAttribute("aria-label", pedal.page.name + " footswitch");
    stomp.addEventListener("click", () => {
      const next = valueOf(pedal.engaged) >= 0.5 ? 0 : 1;
      write(pedal.engaged, next);
      paintEngaged(pedal);
      scheduleLayout();
    });
    const slot = document.createElement("span");
    slot.className = "label slot";
    footer.append(lamp, stomp, slot);

    card.append(head, circuit, knobs, footer);
    attachDrag(card, pedal, head);

    state.cards.set(pedal.page.id, { card: card, stomp: stomp, slot: slot, moves: [earlier, later] });
    state.controls.set(pedal.engaged.index, { apply: () => paintEngaged(pedal) });
    state.controls.set(pedal.position.index, { apply: () => scheduleLayout() });
    return card;
  }

  function paintEngaged(pedal) {
    const entry = state.cards.get(pedal.page.id);
    if (!entry) return;
    const engaged = valueOf(pedal.engaged) >= 0.5;
    entry.card.classList.toggle("on", engaged);
    entry.card.classList.toggle("off", !engaged);
    entry.stomp.setAttribute("aria-pressed", String(engaged));
  }

  let layoutFrame = null;
  let layoutFallback = null;
  /**
   * Coalesces layout requests to one per frame — but never *waits* for a frame
   * that may not come. A surface behind another panel gets no animation frames
   * at all, and a board that only lays itself out inside `requestAnimationFrame`
   * would still be empty when it came back into view.
   */
  function scheduleLayout() {
    if (layoutFrame !== null || layoutFallback !== null) return;
    const run = () => {
      if (layoutFrame !== null) cancelAnimationFrame(layoutFrame);
      clearTimeout(layoutFallback);
      layoutFrame = null;
      layoutFallback = null;
      layout();
    };
    layoutFrame = requestAnimationFrame(run);
    layoutFallback = setTimeout(run, 50);
  }

  /** Puts the existing cards in board order without rebuilding them. */
  function layout() {
    const order = boardOrder();
    const fragment = document.createDocumentFragment();
    order.forEach((pedal, index) => {
      if (index > 0) fragment.appendChild(cable());
      const entry = state.cards.get(pedal.page.id);
      fragment.appendChild(entry.card);
      entry.slot.textContent = index + 1 + " / " + order.length;
      entry.moves[0].disabled = index === 0;
      entry.moves[1].disabled = index === order.length - 1;
      paintEngaged(pedal);
    });
    boardElement.textContent = "";
    boardElement.appendChild(fragment);
    publishSurfaceInfo(order);
  }

  let publishedInfo = "";
  function publishSurfaceInfo(order) {
    const engaged = order.filter((pedal) => valueOf(pedal.engaged) >= 0.5).length;
    const value = engaged + " of " + order.length + " on";
    // Layout runs on every reorder, and a drag reorders many times a second.
    // The host only needs to hear about it when the words change.
    if (value === publishedInfo) return;
    publishedInfo = value;
    call("plugin.set_surface_info", { label: "Board", value: value }).catch(
      () => undefined,
    );
  }

  /** Renumbers every position parameter to match the order shown. */
  function commitOrder(order) {
    order.forEach((pedal, index) => write(pedal.position, index + 1));
    scheduleLayout();
    say("Board order updated", false);
  }

  function move(pedal, direction) {
    const order = boardOrder();
    // Cards are rebuilt from the schema, so identity comes from the page.
    const from = order.findIndex((candidate) => candidate.page.id === pedal.page.id);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= order.length) return;
    order.splice(to, 0, order.splice(from, 1)[0]);
    commitOrder(order);
  }

  /* ------------------------------------------------------- drag to reorder */

  /**
   * Which way the board runs. It is a row on a desktop and a column on a
   * phone, and a drag has to follow whichever it is: comparing horizontal
   * positions on a stacked board compares seven identical numbers.
   */
  function boardAxis() {
    const direction = getComputedStyle(boardElement).flexDirection || "row";
    return direction.startsWith("column") ? "vertical" : "horizontal";
  }

  function attachDrag(card, pedal, handle) {
    let dragging = false;
    let pointerId = null;
    let origin = 0;
    let axis = "horizontal";

    const along = (event) => (axis === "vertical" ? event.clientY : event.clientX);
    const middleOf = (rect) =>
      axis === "vertical" ? rect.top + rect.height / 2 : rect.left + rect.width / 2;

    handle.addEventListener("pointerdown", (event) => {
      // Only the header drags, so a knob inside the card keeps its gesture.
      if (event.target.closest("button")) return;
      dragging = true;
      pointerId = event.pointerId;
      axis = boardAxis();
      origin = along(event);
      capture(handle, pointerId);
      card.classList.add("dragging");
      gestures.add(finish);
      event.preventDefault();
    });

    handle.addEventListener("pointermove", (event) => {
      if (!dragging) return;
      const shift = along(event) - origin;
      card.style.transform =
        (axis === "vertical" ? "translateY(" : "translateX(") + shift + "px)";

      const order = boardOrder();
      const index = order.findIndex((candidate) => candidate.page.id === pedal.page.id);
      const centre = middleOf(card.getBoundingClientRect());

      // Compare against the neighbour the drag is heading towards.
      const direction = shift > 0 ? 1 : -1;
      const neighbour = order[index + direction];
      if (!neighbour) return;
      const other = middleOf(state.cards.get(neighbour.page.id).card.getBoundingClientRect());
      if (direction > 0 ? centre <= other : centre >= other) return;

      order.splice(index + direction, 0, order.splice(index, 1)[0]);
      origin = along(event);
      card.style.transform = "";
      commitOrder(order);
    });

    function finish() {
      if (!dragging) return;
      dragging = false;
      gestures.delete(finish);
      card.classList.remove("dragging");
      card.style.transform = "";
      releaseCapture(handle, pointerId);
    }

    handle.addEventListener("pointerup", finish);
    handle.addEventListener("pointercancel", finish);
    handle.addEventListener("lostpointercapture", finish);
  }

  /* ----------------------------------------------------------------- rig */

  function buildRig() {
    rigElement.textContent = "";
    parametersOfPage(RIG_PAGE).forEach((parameter) => {
      rigElement.appendChild(control(parameter));
    });
  }

  /* ------------------------------------------------------------- presets */

  function renderPresetList() {
    const current = presetElement.value;
    presetElement.textContent = "";
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = state.sounds.length ? "Factory boards…" : "No factory boards";
    presetElement.appendChild(placeholder);
    state.sounds.forEach((sound) => {
      const option = document.createElement("option");
      option.value = sound.id;
      option.textContent = sound.name;
      option.title = sound.detail || "";
      presetElement.appendChild(option);
    });
    presetElement.value = state.selectedSoundId || current || "";
  }

  presetElement.addEventListener("change", () => {
    const soundId = presetElement.value;
    if (!soundId) return;
    call("plugin.select_sound", { sound_id: soundId })
      .then(() => {
        state.selectedSoundId = soundId;
        state.edited = false;
        say("Loaded " + presetElement.selectedOptions[0].textContent, false);
        return refresh();
      })
      .catch((error) => say("Could not load that board: " + error.message, true));
  });

  /* --------------------------------------------------------------- wiring */

  function applyValues(values, readEpoch) {
    (values || []).forEach((entry) => {
      // A control under the hand owns its value, whatever the host thinks.
      if (state.held.has(entry.index)) return;
      // And a value written after this read went out describes a later moment
      // than the reply does.
      if ((state.writtenAt.get(entry.index) || 0) > readEpoch) return;
      state.values.set(entry.index, entry.value);
      const widget = state.controls.get(entry.index);
      if (widget) widget.apply(entry.value);
    });
  }

  function scheduleRefresh(delay) {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, delay === undefined ? 150 : delay);
  }

  async function refresh() {
    if (state.queue.size > 0 || writing || state.held.size > 0) {
      // Reading now would answer with values this page has already replaced,
      // or overwrite one that a hand is still on.
      scheduleRefresh();
      return;
    }
    const readEpoch = writeEpoch;
    try {
      const parameters = await call("plugin.parameters");
      const schemaChanged =
        !state.schema ||
        state.schema.parameters.length !== parameters.schema.parameters.length;
      state.schema = parameters.schema;
      if (!state.built || schemaChanged) {
        build();
      }
      applyValues(parameters.values, readEpoch);
      scheduleLayout();
      idle();
    } catch (error) {
      say("RackForge did not answer: " + error.message, true);
    }
  }

  function build() {
    state.controls.clear();
    state.cards.clear();
    // Indices are only meaningful within one schema.
    state.writtenAt.clear();
    state.pedals = collectPedals();
    buildRig();
    boardElement.textContent = "";
    state.pedals.forEach((pedal) => pedalCard(pedal));
    state.built = true;
  }

  parent.postMessage({ protocol: PROTOCOL, kind: "ready" }, "*");
})();

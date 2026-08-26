/**
 * RF-Rig PLAY surface.
 *
 * The page builds itself from the parameter schema the host sends back, so it
 * has no private list of pedals or knobs: adding a control in the Rust contract
 * makes it appear here. The one convention it relies on is the naming inside a
 * page — `<pedal>.engaged` is the footswitch, `<pedal>.position` is where the
 * pedal sits on the board, everything else is a control.
 */
(function () {
  "use strict";

  const PROTOCOL = "rackforge.plugin.web@1";
  const RIG_PAGE = "rig";

  const pending = new Map();
  let nextRequest = 1;
  let schema = null;
  const values = new Map();
  let surface = "play";
  let refreshTimer = null;
  let selectedSoundId = "";

  const boardElement = document.getElementById("board");
  const rigElement = document.getElementById("rig");
  const presetElement = document.getElementById("presets");
  const statusElement = document.getElementById("status");

  function say(text, isError) {
    statusElement.textContent = text;
    statusElement.classList.toggle("error", Boolean(isError));
  }

  function call(method, params) {
    const requestId = "rf-rig-" + nextRequest++;
    return new Promise((resolve, reject) => {
      pending.set(requestId, { resolve, reject });
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
      surface = message.surface || "play";
      const instance = message.instance || {};
      selectedSoundId = instance.selected_sound_id || selectedSoundId;
      renderPresetList(instance.sounds || []);
      // The host sends a fresh context whenever the session revision moves,
      // which is also how this surface learns that something else changed the
      // board. Coalesce, or a burst of revisions would mean a burst of reads.
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => void load(), 60);
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

  /* ---------------------------------------------------------------- model */

  function parametersOfPage(pageId) {
    return schema.parameters
      .filter((parameter) => parameter.page === pageId)
      .sort((left, right) => (left.order || 0) - (right.order || 0));
  }

  function findByIdSuffix(pageId, suffix) {
    return schema.parameters.find((parameter) => parameter.id === pageId + "." + suffix);
  }

  function pedalPages() {
    return schema.pages
      .filter((page) => page.id !== RIG_PAGE)
      .map((page) => ({
        page: page,
        engaged: findByIdSuffix(page.id, "engaged"),
        position: findByIdSuffix(page.id, "position"),
      }))
      .filter((pedal) => pedal.engaged && pedal.position);
  }

  function boardOrder() {
    const pedals = pedalPages();
    return pedals
      .map((pedal, declaration) => ({ pedal: pedal, declaration: declaration }))
      .sort((left, right) => {
        const difference =
          (values.get(left.pedal.position.index) || 0) -
          (values.get(right.pedal.position.index) || 0);
        return difference !== 0 ? difference : left.declaration - right.declaration;
      })
      .map((entry) => entry.pedal);
  }

  function valueOf(parameter) {
    const stored = values.get(parameter.index);
    if (stored !== undefined) return stored;
    const kind = parameter.kind;
    if (kind.type === "boolean") return kind.default ? 1 : 0;
    return kind.default;
  }

  async function setValue(parameter, value) {
    values.set(parameter.index, value);
    try {
      await call("plugin.set_parameter", {
        parameter_index: parameter.index,
        value: value,
      });
    } catch (error) {
      say(parameter.name + ": " + error.message, true);
      throw error;
    }
  }

  /* ------------------------------------------------------------ rendering */

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

    const track = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    track.setAttribute("cx", String(centre));
    track.setAttribute("cy", String(centre));
    track.setAttribute("r", String(radius));
    track.setAttribute("fill", "#1e2124");
    track.setAttribute("stroke", "#b0b2b4");
    track.setAttribute("stroke-width", "2");
    svg.appendChild(track);

    const pointer = document.createElementNS("http://www.w3.org/2000/svg", "line");
    pointer.setAttribute("stroke", "#e8a33d");
    pointer.setAttribute("stroke-width", "3");
    pointer.setAttribute("stroke-linecap", "round");
    svg.appendChild(pointer);

    const label = document.createElement("div");
    label.className = "label";
    label.textContent = parameter.name;
    const reading = document.createElement("div");
    reading.className = "reading";

    function normalised(value) {
      return (value - kind.minimum) / (kind.maximum - kind.minimum);
    }

    function paint(value) {
      // A real pot sweeps about 270 degrees, from lower left to lower right.
      const angle = (-225 + 270 * normalised(value)) * (Math.PI / 180);
      pointer.setAttribute("x1", String(centre));
      pointer.setAttribute("y1", String(centre));
      pointer.setAttribute("x2", String(centre + Math.cos(angle) * radius * 0.78));
      pointer.setAttribute("y2", String(centre + Math.sin(angle) * radius * 0.78));
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
    let startY = 0;
    let startValue = 0;
    let queued = null;

    function flush() {
      if (queued === null) return;
      const value = queued;
      queued = null;
      void setValue(parameter, value);
    }

    svg.addEventListener("pointerdown", (event) => {
      dragging = true;
      startY = event.clientY;
      startValue = valueOf(parameter);
      svg.setPointerCapture(event.pointerId);
      event.preventDefault();
    });

    svg.addEventListener("pointermove", (event) => {
      if (!dragging) return;
      // Two hundred pixels of travel covers the whole range; holding shift
      // slows it down for the settings that actually need precision.
      const scale = event.shiftKey ? 800 : 200;
      const delta = ((startY - event.clientY) / scale) * (kind.maximum - kind.minimum);
      const value = quantise(startValue + delta);
      paint(value);
      values.set(parameter.index, value);
      if (queued === null) requestAnimationFrame(flush);
      queued = value;
    });

    function release(event) {
      if (!dragging) return;
      dragging = false;
      try {
        svg.releasePointerCapture(event.pointerId);
      } catch (error) {
        /* the capture is already gone */
      }
      flush();
    }

    svg.addEventListener("pointerup", release);
    svg.addEventListener("pointercancel", release);

    svg.addEventListener("dblclick", () => {
      const value = kind.default;
      paint(value);
      void setValue(parameter, value);
    });

    svg.addEventListener("keydown", (event) => {
      const step = (kind.maximum - kind.minimum) / (event.shiftKey ? 200 : 20);
      let value = valueOf(parameter);
      if (event.key === "ArrowUp" || event.key === "ArrowRight") value += step;
      else if (event.key === "ArrowDown" || event.key === "ArrowLeft") value -= step;
      else return;
      event.preventDefault();
      value = quantise(value);
      paint(value);
      void setValue(parameter, value);
    });

    paint(valueOf(parameter));
    wrapper.append(svg, label, reading);
    return wrapper;
  }

  function choiceControl(parameter) {
    const wrapper = document.createElement("div");
    wrapper.className = "knob";
    const label = document.createElement("div");
    label.className = "label";
    label.textContent = parameter.name;
    const row = document.createElement("div");
    row.className = "choice";

    parameter.kind.choices.forEach((choice) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = choice.name;
      button.setAttribute(
        "aria-pressed",
        String(Math.round(valueOf(parameter)) === choice.value),
      );
      button.addEventListener("click", () => {
        void setValue(parameter, choice.value).then(() => {
          Array.from(row.children).forEach((sibling, index) => {
            sibling.setAttribute(
              "aria-pressed",
              String(parameter.kind.choices[index].value === choice.value),
            );
          });
        });
      });
      row.appendChild(button);
    });

    wrapper.append(label, row);
    return wrapper;
  }

  function control(parameter) {
    if (parameter.kind.type === "enum") return choiceControl(parameter);
    return knobControl(parameter);
  }

  function cable() {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("class", "cable");
    svg.setAttribute("viewBox", "0 0 34 60");
    svg.setAttribute("aria-hidden", "true");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M0 18 C 10 18, 12 40, 17 40 C 22 40, 24 18, 34 18");
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", "#3a3531");
    path.setAttribute("stroke-width", "5");
    path.setAttribute("stroke-linecap", "round");
    svg.appendChild(path);
    return svg;
  }

  /** Renumbers every position parameter to match the order shown. */
  async function commitOrder(order) {
    const writes = [];
    order.forEach((pedal, index) => {
      const position = index + 1;
      if (valueOf(pedal.position) !== position) {
        writes.push(setValue(pedal.position, position));
      }
    });
    try {
      await Promise.all(writes);
      say("Board order updated", false);
    } catch (error) {
      say("The host refused the new order: " + error.message, true);
    }
    render();
  }

  function move(pedal, direction) {
    const order = boardOrder();
    // Pedals are rebuilt from the schema on every render, so identity has to
    // come from the page they belong to rather than from the object.
    const from = order.findIndex((candidate) => candidate.page.id === pedal.page.id);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= order.length) return;
    order.splice(to, 0, order.splice(from, 1)[0]);
    void commitOrder(order);
  }

  function pedalCard(pedal, index, total) {
    const card = document.createElement("section");
    const engaged = valueOf(pedal.engaged) >= 0.5;
    card.className = "pedal " + (engaged ? "on" : "off");
    card.setAttribute("aria-label", pedal.page.name);

    const head = document.createElement("div");
    head.className = "pedal-head";
    const name = document.createElement("span");
    name.className = "pedal-name";
    name.textContent = pedal.page.name.toUpperCase();
    const moveBox = document.createElement("div");
    moveBox.className = "move";
    const earlier = document.createElement("button");
    earlier.type = "button";
    earlier.textContent = "◀";
    earlier.title = "Move earlier in the chain";
    earlier.disabled = index === 0;
    earlier.addEventListener("click", () => move(pedal, -1));
    const later = document.createElement("button");
    later.type = "button";
    later.textContent = "▶";
    later.title = "Move later in the chain";
    later.disabled = index === total - 1;
    later.addEventListener("click", () => move(pedal, 1));
    moveBox.append(earlier, later);
    head.append(name, moveBox);

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
    stomp.setAttribute("aria-pressed", String(engaged));
    stomp.setAttribute("aria-label", pedal.page.name + " footswitch");
    stomp.addEventListener("click", () => {
      const next = engaged ? 0 : 1;
      void setValue(pedal.engaged, next).then(render);
    });
    const slot = document.createElement("span");
    slot.className = "label";
    slot.textContent = index + 1 + " / " + total;
    footer.append(lamp, stomp, slot);

    card.append(head, circuit, knobs, footer);
    return card;
  }

  function renderRig() {
    rigElement.textContent = "";
    parametersOfPage(RIG_PAGE).forEach((parameter) => {
      rigElement.appendChild(control(parameter));
    });
  }

  function render() {
    if (!schema) return;
    renderRig();
    boardElement.textContent = "";
    const order = boardOrder();
    order.forEach((pedal, index) => {
      if (index > 0) boardElement.appendChild(cable());
      boardElement.appendChild(pedalCard(pedal, index, order.length));
    });
    const engaged = order.filter((pedal) => valueOf(pedal.engaged) >= 0.5).length;
    void call("plugin.set_surface_info", {
      label: "Board",
      value: engaged + " of " + order.length + " engaged",
    }).catch(() => undefined);
  }

  /* --------------------------------------------------------------- wiring */

  function renderPresetList(presets) {
    presetElement.textContent = "";
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = "Factory boards…";
    presetElement.appendChild(placeholder);
    presets.forEach((preset) => {
      const option = document.createElement("option");
      option.value = preset.id;
      option.textContent = preset.name;
      option.title = preset.detail || "";
      option.selected = preset.id === selectedSoundId;
      presetElement.appendChild(option);
    });
    if (!presets.length) {
      placeholder.textContent = "No factory boards";
    }
  }

  presetElement.addEventListener("change", () => {
    const soundId = presetElement.value;
    if (!soundId) return;
    call("plugin.select_sound", { sound_id: soundId })
      .then(() => {
        say("Loaded " + presetElement.selectedOptions[0].textContent, false);
        return load();
      })
      .catch((error) => say("Could not load that board: " + error.message, true));
  });

  async function load() {
    try {
      const parameters = await call("plugin.parameters");
      schema = parameters.schema;
      values.clear();
      (parameters.values || []).forEach((entry) => values.set(entry.index, entry.value));
      render();
      say("Connected · " + surface + " surface", false);
    } catch (error) {
      say("RackForge did not answer: " + error.message, true);
    }
  }

  parent.postMessage({ protocol: PROTOCOL, kind: "ready" }, "*");
})();

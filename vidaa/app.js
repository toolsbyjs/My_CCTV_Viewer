/* MyCCTV TV app (Hisense VIDAA OS and other TV browsers).
 * Talks to the "TV server" in MyCCTV (desktop or Android box). Written in plain ES5 so older
 * TV browsers run it. Remote: arrows, OK, Back, numbers and the colour keys. */
(function () {
  "use strict";

  var KEY = { LEFT: 37, UP: 38, RIGHT: 39, DOWN: 40, OK: 13, BACK: [8, 27, 461, 10009], GREEN: 404, YELLOW: 405, RED: 403, BLUE: 406 };
  var store = window.localStorage;
  var state = {
    server: store.getItem("mycctv.server") || "",
    key: store.getItem("mycctv.key") || "",
    layout: parseInt(store.getItem("mycctv.layout") || "4", 10),
    tiles: JSON.parse(store.getItem("mycctv.tiles") || "[]"),
    sel: 0,
    full: false,
    ptz: false,
    devices: [],
    pickerFocus: 0
  };

  function $(id) { return document.getElementById(id); }
  function base() {
    // Served by MyCCTV itself: same address. Opened from elsewhere (packaged app): the saved address.
    if (location.protocol === "http:" && location.port) { return ""; }
    return state.server.replace(/\/+$/, "");
  }
  function save() {
    store.setItem("mycctv.server", state.server);
    store.setItem("mycctv.key", state.key);
    store.setItem("mycctv.layout", String(state.layout));
    store.setItem("mycctv.tiles", JSON.stringify(state.tiles));
  }
  function get(path, done, fail) {
    var x = new XMLHttpRequest();
    x.open("GET", base() + path + (path.indexOf("?") < 0 ? "?" : "&") + "key=" + encodeURIComponent(state.key), true);
    x.timeout = 8000;
    x.onload = function () {
      if (x.status === 200) { try { done(JSON.parse(x.responseText)); } catch (e) { fail("bad answer"); } }
      else if (x.status === 403) { fail("The access code is not right"); }
      else { fail("MyCCTV answered " + x.status); }
    };
    x.onerror = x.ontimeout = function () { fail("Can't reach MyCCTV at this address"); };
    x.send();
  }

  /* ---------- setup ---------- */
  function showSetup(err) {
    $("setup").className = "overlay";
    $("serverRow").style.display = base() === "" && location.protocol === "http:" ? "none" : "block";
    $("server").value = state.server;
    $("key").value = state.key;
    $("setupError").textContent = err || "";
    (state.key ? $("btnConnect") : $("key")).focus();
  }
  $("btnConnect").onclick = function () {
    state.server = $("server").value.trim();
    if (state.server && state.server.indexOf("://") < 0) { state.server = "http://" + state.server; }
    state.key = $("key").value.trim();
    save();
    connect();
  };
  function connect() {
    get("/api/devices", function (list) {
      state.devices = list;
      $("setup").className = "overlay hidden";
      if (!state.tiles.length && list.length) {
        // First time: fill the windows with the first cameras.
        var n = 0;
        for (var i = 0; i < list.length; i++) {
          for (var j = 0; j < list[i].cameras.length && n < state.layout; j++) { state.tiles[n++] = { dev: list[i].id, ch: list[i].cameras[j].ch }; }
        }
        save();
      }
      render();
    }, function (err) { showSetup(err); });
  }

  /* ---------- wall ---------- */
  function camName(t) {
    for (var i = 0; i < state.devices.length; i++) {
      var d = state.devices[i];
      if (d.id === t.dev) {
        for (var j = 0; j < d.cameras.length; j++) { if (d.cameras[j].ch === t.ch) { return d.name + " · " + d.cameras[j].name; } }
      }
    }
    return null;
  }
  function render() {
    var wall = $("wall");
    var cols = state.layout === 1 ? 1 : state.layout === 4 ? 2 : 3;
    wall.style.gridTemplateColumns = "repeat(" + cols + ", 1fr)";
    wall.style.gridTemplateRows = "repeat(" + cols + ", 1fr)";
    wall.className = state.full ? "full" : "";
    // Close old video streams before building new tiles.
    var imgs = wall.getElementsByTagName("img");
    for (var k = 0; k < imgs.length; k++) { imgs[k].src = ""; }
    wall.innerHTML = "";
    if (state.sel >= state.layout) { state.sel = 0; }
    for (var i = 0; i < state.layout; i++) {
      var tile = document.createElement("div");
      tile.className = "tile" + (i === state.sel ? " sel" : "") + (i === state.sel && state.ptz ? " ptz" : "");
      tile.setAttribute("data-i", i);
      var t = state.tiles[i];
      var name = t ? camName(t) : null;
      if (t && name) {
        var img = document.createElement("img");
        var sharp = state.layout === 1 || (state.full && i === state.sel);
        img.src = base() + "/mjpeg?key=" + encodeURIComponent(state.key) + "&dev=" + t.dev + "&ch=" + t.ch + "&s=" + (sharp ? 0 : 1);
        img.onerror = (function (tl) { return function () { addMsg(tl, "No video (camera offline?)"); }; })(tile);
        tile.appendChild(img);
        var label = document.createElement("div");
        label.className = "name";
        label.innerHTML = "<i>LIVE</i>";
        label.appendChild(document.createTextNode(name));
        tile.appendChild(label);
      } else {
        var e = document.createElement("div");
        e.className = "empty";
        e.innerHTML = "<b>+</b>Press OK to choose a camera";
        tile.appendChild(e);
      }
      tile.onclick = (function (n) { return function () { state.sel = n; openPicker(); }; })(i);
      wall.appendChild(tile);
    }
    var lays = document.querySelectorAll(".lay");
    for (var j = 0; j < lays.length; j++) { lays[j].className = "lay" + (parseInt(lays[j].getAttribute("data-n"), 10) === state.layout ? " on" : ""); }
    $("ptz").className = state.ptz ? "" : "hidden";
  }
  function addMsg(tile, text) {
    var m = document.createElement("div");
    m.className = "msg";
    m.textContent = text;
    tile.appendChild(m);
  }
  function setLayout(n) { state.layout = n; state.full = false; save(); render(); }
  function moveSel(dx, dy) {
    var cols = state.layout === 1 ? 1 : state.layout === 4 ? 2 : 3;
    var r = Math.floor(state.sel / cols), c = state.sel % cols;
    r = Math.max(0, Math.min(cols - 1, r + dy));
    c = Math.max(0, Math.min(cols - 1, c + dx));
    state.sel = r * cols + c;
    var tiles = document.querySelectorAll(".tile");
    for (var i = 0; i < tiles.length; i++) { tiles[i].className = "tile" + (i === state.sel ? " sel" : ""); }
  }

  /* ---------- camera picker ---------- */
  function openPicker() {
    var list = $("camlist");
    list.innerHTML = "";
    for (var i = 0; i < state.devices.length; i++) {
      var d = state.devices[i];
      var g = document.createElement("div");
      g.className = "group";
      g.textContent = d.name;
      list.appendChild(g);
      for (var j = 0; j < d.cameras.length; j++) {
        var b = document.createElement("button");
        b.className = "item";
        b.textContent = d.cameras[j].name;
        b.onclick = (function (dev, ch) { return function () { choose({ dev: dev, ch: ch }); }; })(d.id, d.cameras[j].ch);
        list.appendChild(b);
      }
    }
    $("picker").className = "overlay";
    var first = list.getElementsByTagName("button")[0] || $("btnClear");
    first.focus();
  }
  function closePicker() { $("picker").className = "overlay hidden"; }
  function choose(t) { state.tiles[state.sel] = t; save(); closePicker(); render(); }
  $("btnClear").onclick = function () { state.tiles[state.sel] = null; save(); closePicker(); render(); };
  function pickerMove(d) {
    var items = $("picker").getElementsByTagName("button");
    var idx = 0;
    for (var i = 0; i < items.length; i++) { if (items[i] === document.activeElement) { idx = i; } }
    idx = Math.max(0, Math.min(items.length - 1, idx + d));
    items[idx].focus();
    if (items[idx].scrollIntoView) { items[idx].scrollIntoView(false); }
  }

  /* ---------- camera control ---------- */
  var ptzHeld = null;
  function ptz(cmd, start) {
    var t = state.tiles[state.sel];
    if (!t) { return; }
    get("/api/ptz?dev=" + t.dev + "&ch=" + t.ch + "&cmd=" + cmd + "&start=" + (start ? 1 : 0), function () {}, function () {});
  }

  /* ---------- keys ---------- */
  function isBack(k) { return KEY.BACK.indexOf(k) >= 0; }
  document.addEventListener("keydown", function (e) {
    var k = e.keyCode;
    var typing = document.activeElement && document.activeElement.tagName === "INPUT";
    if ($("setup").className.indexOf("hidden") < 0) {
      if (k === KEY.OK && !typing) { return; }
      if (k === KEY.DOWN || k === KEY.UP) {
        var f = [$("server"), $("key"), $("btnConnect")].filter(function (el) { return el.offsetParent !== null; });
        var i = f.indexOf(document.activeElement);
        f[Math.max(0, Math.min(f.length - 1, i + (k === KEY.DOWN ? 1 : -1)))].focus();
        e.preventDefault();
      }
      return;
    }
    if ($("picker").className.indexOf("hidden") < 0) {
      if (k === KEY.DOWN || k === KEY.RIGHT) { pickerMove(1); e.preventDefault(); }
      else if (k === KEY.UP || k === KEY.LEFT) { pickerMove(-1); e.preventDefault(); }
      else if (isBack(k)) { closePicker(); e.preventDefault(); }
      return;
    }
    if (state.ptz) {
      var map = {}; map[KEY.UP] = "up"; map[KEY.DOWN] = "down"; map[KEY.LEFT] = "left"; map[KEY.RIGHT] = "right"; map[49] = "zoomout"; map[51] = "zoomin";
      if (map[k]) { if (ptzHeld !== map[k]) { ptzHeld = map[k]; ptz(map[k], true); } e.preventDefault(); return; }
      if (isBack(k) || k === KEY.YELLOW || k === 80) { state.ptz = false; render(); e.preventDefault(); }
      return;
    }
    if (k === KEY.LEFT) { moveSel(-1, 0); }
    else if (k === KEY.RIGHT) { moveSel(1, 0); }
    else if (k === KEY.UP) { moveSel(0, -1); }
    else if (k === KEY.DOWN) { moveSel(0, 1); }
    else if (k === KEY.OK) { if (document.activeElement && document.activeElement.tagName === "BUTTON") { return; } openPicker(); }
    else if (k === KEY.GREEN || k === 70) { state.full = !state.full; render(); }
    else if (k === KEY.YELLOW || k === 80) { if (state.tiles[state.sel]) { state.ptz = true; render(); } }
    else if (k === 49) { setLayout(1); }
    else if (k === 52) { setLayout(4); }
    else if (k === 57) { setLayout(9); }
    else if (isBack(k)) { if (state.full) { state.full = false; render(); } else { return; } }
    else { return; }
    e.preventDefault();
  });
  document.addEventListener("keyup", function (e) {
    if (state.ptz && ptzHeld) { ptz(ptzHeld, false); ptzHeld = null; }
  });

  /* ---------- buttons, clock, start ---------- */
  var lays = document.querySelectorAll(".lay");
  for (var i = 0; i < lays.length; i++) {
    lays[i].onclick = (function (n) { return function () { setLayout(n); this.blur(); }; })(parseInt(lays[i].getAttribute("data-n"), 10));
  }
  $("btnCams").onclick = function () { this.blur(); openPicker(); };
  function tick() {
    var d = new Date();
    $("clock").textContent = ("0" + d.getHours()).slice(-2) + ":" + ("0" + d.getMinutes()).slice(-2);
  }
  setInterval(tick, 10000);
  tick();
  // Access code from the address (?key=1234) saves typing it with the remote.
  var m = /[?&]key=([^&]+)/.exec(location.search);
  if (m) { state.key = decodeURIComponent(m[1]); save(); }
  if (!state.key) { showSetup(""); } else { connect(); }
})();

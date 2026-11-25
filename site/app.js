// MyCCTV website: screen tabs, and download buttons filled in from the newest GitHub release.
(function () {
  "use strict";
  var REPO = "toolsbyjs/My_CCTV_Viewer";

  // Screens: switch the big picture.
  var img = document.getElementById("gallery-img");
  var tabs = document.querySelectorAll(".tabs button");
  var alts = {
    "desktop-live": "Live view with four windows and the camera list",
    "desktop-playback": "Playback with the 24-hour timeline and speed buttons",
    "desktop-devices": "Device cards with status, model, Watch, Edit and Settings",
    "desktop-home": "Dashboard with devices online, cameras and recent events"
  };
  tabs.forEach(function (b) {
    b.addEventListener("click", function () {
      tabs.forEach(function (t) { t.setAttribute("aria-selected", t === b ? "true" : "false"); });
      img.src = "img/" + b.dataset.shot + ".png";
      img.alt = alts[b.dataset.shot] || "";
    });
  });

  // Which platform is this visitor on?
  var ua = navigator.userAgent;
  var mine = /Android/i.test(ua) ? "android" : /Windows/i.test(ua) ? "windows" : /Macintosh|Mac OS X/i.test(ua) ? "macos" : null;
  if (mine) {
    var card = document.querySelector('.dl[data-kind="' + mine + '"]');
    if (card) card.classList.add("suggested");
  }

  function size(bytes) {
    return (bytes / 1048576).toFixed(1) + " MB";
  }

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", { headers: { Accept: "application/vnd.github+json" } })
    .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(function (rel) {
      var version = (rel.tag_name || "").replace(/^v/, "");
      var date = rel.published_at ? new Date(rel.published_at).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" }) : "";
      document.getElementById("release-line").textContent = "Version " + version + (date ? ", released " + date : "") + ".";
      var found = {};
      (rel.assets || []).forEach(function (a) {
        var kind = /windows/i.test(a.name) ? "windows" : /android/i.test(a.name) ? "android" : /macos/i.test(a.name) ? "macos" : null;
        if (kind) found[kind] = a;
      });
      document.querySelectorAll(".dl").forEach(function (card) {
        var a = found[card.dataset.kind];
        var meta = card.querySelector(".meta");
        if (a) {
          card.querySelector(".btn").href = a.browser_download_url;
          meta.textContent = a.name + " · " + size(a.size);
        } else {
          card.classList.add("missing");
          meta.textContent = "Not in this release yet: see all releases.";
        }
      });
      var main = document.getElementById("main-download");
      if (mine && found[mine]) {
        var names = { windows: "Windows", android: "Android", macos: "Mac" };
        main.href = found[mine].browser_download_url;
        main.textContent = "Download for " + names[mine];
      }
      document.getElementById("version-line").textContent = "Version " + version + " · H.264 and H.265 cameras · RTSP · Dahua-compatible recorders";
    })
    .catch(function () {
      // No release yet, offline, or the API limit: the buttons keep pointing at the releases page.
      document.getElementById("release-line").textContent = "Downloads are on the GitHub releases page.";
    });
})();

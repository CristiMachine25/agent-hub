// Baton landing page: segmented meters, the hero handoff demo, and the screen tour tabs.
(function () {
  "use strict";

  var CELLS = 20;
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  function level(left) {
    if (left >= 50) return "full";
    if (left >= 20) return "half";
    if (left >= 5) return "low";
    return "empty";
  }

  function lit(left, cells) {
    if (left <= 0.5) return 0;
    return Math.min(cells, Math.max(1, Math.round((left / 100) * cells)));
  }

  /** Fill a .meter element with cells once, then light `left` percent of them. */
  function setMeter(el, left, cells) {
    cells = cells || CELLS;
    if (el.children.length !== cells) {
      el.textContent = "";
      for (var i = 0; i < cells; i++) el.appendChild(document.createElement("i"));
    }
    var n = lit(left, cells);
    el.dataset.level = level(left);
    for (var j = 0; j < cells; j++) el.children[j].classList.toggle("on", j < n);
  }

  document.querySelectorAll(".meter[data-static]").forEach(function (el) {
    setMeter(el, Number(el.dataset.static), 12);
  });

  // ---------- hero board ----------
  var board = document.getElementById("board");
  if (board) {
    var accounts = [
      { name: "H2 Max", tool: "Claude Code, Max 5x", color: "#199E70", left: 64, cmd: "CLAUDE_CONFIG_DIR=~/.claude-h2 claude", verb: "Launch Claude", short: "Claude" },
      { name: "Work Max 20x", tool: "Claude Code, Max 20x", color: "#3987E5", left: 88, cmd: "CLAUDE_CONFIG_DIR=~/.claude-work claude --continue", verb: "Launch Claude", short: "Claude" },
      { name: "ChatGPT Pro", tool: "Codex, Pro", color: "#C98500", left: 71, cmd: "CODEX_HOME=~/.codex-pro codex resume --last", verb: "Launch Codex", short: "Codex" },
      { name: "Personal Pro", tool: "Claude Code, Pro", color: "#D95926", left: 9, cmd: "CLAUDE_CONFIG_DIR=~/.claude-personal claude", verb: "Launch Claude", short: "Claude" },
      { name: "Gemini AI Pro", tool: "Gemini CLI", color: "#9085E9", left: 52, cmd: "GEMINI_CLI_HOME=~/.gemini-ai-pro gemini", verb: "Launch Gemini", short: "Gemini" }
    ];
    var active = 0;
    var minutes = 14 * 60 + 20; // simulated clock, Tue 14:20
    var resetAt = accounts.map(function (_, i) { return 150 + i * 70; }); // minutes until each 5-hour window resets
    var $ = function (sel) { return board.querySelector(sel); };
    var rowsEl = $("[data-rows]");
    var bigMeter = $("[data-best-meter]");
    var toast = $("[data-toast]");
    var toastTimer = 0;

    accounts.forEach(function (a) {
      var li = document.createElement("li");
      li.innerHTML = '<span class="dot"></span><span class="row-name"></span><span class="meter"></span><span class="row-pct"></span>';
      li.querySelector(".dot").style.setProperty("--c", a.color);
      li.querySelector(".row-name").innerHTML = a.name + "<small>" + a.short + "</small>";
      rowsEl.appendChild(li);
      a.li = li;
    });

    var fmt = function (m) {
      var h = Math.floor(m / 60), mm = Math.round(m % 60);
      return h ? h + "h " + (mm ? mm + "m" : "") : mm + "m";
    };

    function render() {
      var a = accounts[active];
      $("[data-best-name]").textContent = a.name;
      $("[data-best-tool]").textContent = a.tool;
      $("[data-best-dot]").style.setProperty("--c", a.color);
      var pct = $("[data-best-pct]");
      pct.textContent = Math.round(a.left) + "%";
      pct.dataset.level = level(a.left);
      setMeter(bigMeter, a.left);
      $("[data-best-reset]").textContent = "Resets in " + fmt(resetAt[active]);
      $("[data-best-btn]").textContent = a.verb;
      var d = Math.floor(minutes / 1440) % 7, t = minutes % 1440;
      $("[data-clock]").textContent = ["Tue", "Wed", "Thu", "Fri", "Sat", "Sun", "Mon"][d] + " " +
        String(Math.floor(t / 60)).padStart(2, "0") + ":" + String(t % 60).padStart(2, "0");
      accounts.forEach(function (x, i) {
        x.li.classList.toggle("active", i === active);
        setMeter(x.li.querySelector(".meter"), x.left);
        x.li.querySelector(".row-pct").textContent = x.left <= 0.5 ? "At limit" : Math.round(x.left) + "% left";
      });
    }

    /** The next account to pass the work to: the one with the most left. */
    function next() {
      var best = -1;
      accounts.forEach(function (x, i) {
        if (i !== active && (best < 0 || x.left > accounts[best].left)) best = i;
      });
      return best;
    }

    function handoff() {
      var from = accounts[active];
      active = next();
      var to = accounts[active];
      $("[data-toast-msg]").textContent = from.name + " hit its limit. Passed to " + to.name + ".";
      $("[data-toast-cmd]").textContent = to.cmd;
      toast.classList.add("show");
      clearTimeout(toastTimer);
      toastTimer = setTimeout(function () { toast.classList.remove("show"); }, 3200);
    }

    function step() {
      minutes += 3;
      accounts.forEach(function (x, i) {
        resetAt[i] -= 3;
        if (resetAt[i] <= 0) {
          x.left = 100;
          resetAt[i] = 300;
        }
      });
      var a = accounts[active];
      a.left = Math.max(0, a.left - 1.6);
      render();
      if (a.left <= 0) handoff();
    }

    render();
    var timer = 0;
    var paused = reduce;
    var visible = true;
    var pauseBtn = document.querySelector("[data-pause]");
    function sync() {
      clearInterval(timer);
      if (!paused && visible && !document.hidden) timer = setInterval(step, 180);
    }
    if (pauseBtn) {
      if (reduce) pauseBtn.hidden = true;
      pauseBtn.addEventListener("click", function () {
        paused = !paused;
        pauseBtn.setAttribute("aria-pressed", String(paused));
        pauseBtn.textContent = paused ? "Play demo" : "Pause demo";
        sync();
      });
    }
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (es) {
        visible = es[0].isIntersecting;
        sync();
      }).observe(board);
    }
    document.addEventListener("visibilitychange", sync);
    sync();
  }

  // ---------- screen tour tabs ----------
  var tabs = Array.prototype.slice.call(document.querySelectorAll('.tour-list [role="tab"]'));
  function select(tab, focus) {
    tabs.forEach(function (t) {
      var on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
    });
    if (focus) tab.focus();
  }
  tabs.forEach(function (t, i) {
    t.addEventListener("click", function () { select(t); });
    t.addEventListener("keydown", function (e) {
      var d = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
      if (!d) return;
      e.preventDefault();
      select(tabs[(i + d + tabs.length) % tabs.length], true);
    });
  });
})();

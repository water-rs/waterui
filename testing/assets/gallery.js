(() => {
  const suiteFilter = document.getElementById("filter-suite");
  const statusFilter = document.getElementById("filter-status");
  const flipFilter = document.getElementById("filter-flip");
  const searchFilter = document.getElementById("filter-search");
  const viewModeSelect = document.getElementById("view-mode");
  const emptyState = document.getElementById("empty-state");
  const lightbox = document.getElementById("lightbox");
  const lightboxImg = document.getElementById("lightbox-img");
  const lightboxCaption = document.getElementById("lightbox-caption");

  const cards = Array.from(document.querySelectorAll(".stage-card"));
  const sections = Array.from(document.querySelectorAll(".suite-section"));

  function applyFilters() {
    const suite = suiteFilter ? suiteFilter.value : "all";
    const status = statusFilter ? statusFilter.value : "all";
    const minFlip = flipFilter ? Number.parseFloat(flipFilter.value || "0") : 0;
    const query = (searchFilter ? searchFilter.value : "").trim().toLowerCase();
    const viewMode = viewModeSelect ? viewModeSelect.value : "side-by-side";

    let visibleCount = 0;

    for (const card of cards) {
      const cardSuite = card.dataset.suite || "";
      const cardStatus = card.dataset.status || "";
      const cardFlip = Number.parseFloat(card.dataset.flipMean || "0");
      const cardSearch = (card.dataset.search || "").toLowerCase();
      const hasComparison = card.dataset.hasComparison === "true";

      const matchesSuite = suite === "all" || cardSuite === suite;
      const matchesStatus = status === "all" || cardStatus === status;
      const matchesFlip = Number.isNaN(minFlip) || minFlip <= 0 || (hasComparison && cardFlip >= minFlip);
      const matchesSearch = query.length === 0 || cardSearch.includes(query);

      const visible = matchesSuite && matchesStatus && matchesFlip && matchesSearch;
      card.classList.toggle("hidden", !visible);

      if (hasComparison) {
        card.dataset.viewMode = viewMode;
      } else {
        card.dataset.viewMode = "side-by-side";
      }

      if (visible) {
        visibleCount += 1;
      }
    }

    for (const section of sections) {
      const anyVisible = section.querySelector(".stage-card:not(.hidden)") !== null;
      section.style.display = anyVisible ? "" : "none";
    }

    if (emptyState) {
      emptyState.style.display = visibleCount === 0 ? "block" : "none";
    }
  }

  for (const el of [suiteFilter, statusFilter, flipFilter, searchFilter, viewModeSelect]) {
    if (el) {
      el.addEventListener("input", applyFilters);
      el.addEventListener("change", applyFilters);
    }
  }

  for (const btn of document.querySelectorAll(".toggle-btn")) {
    btn.addEventListener("click", () => {
      const card = btn.closest(".stage-card");
      if (!card) return;
      const next = card.dataset.toggleSide === "reference" ? "candidate" : "reference";
      card.dataset.toggleSide = next;
      btn.textContent = next === "reference" ? "Showing Reference (Switch)" : "Showing Candidate (Switch)";
    });
  }

  for (const img of document.querySelectorAll(".image-frame img")) {
    img.addEventListener("click", () => {
      if (!lightbox || !lightboxImg || !lightboxCaption) return;
      lightboxImg.src = img.src;
      lightboxCaption.textContent = img.alt || "";
      lightbox.classList.add("open");
    });
  }

  if (lightbox) {
    lightbox.addEventListener("click", () => {
      lightbox.classList.remove("open");
    });
  }

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && lightbox) {
      lightbox.classList.remove("open");
    }
  });

  applyFilters();
})();

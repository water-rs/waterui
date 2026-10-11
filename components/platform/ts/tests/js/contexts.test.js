// The environment contexts: seeded by the host at mount, resolvable through
// component boundaries, and live through host-owned subscriptions.

import { beforeEach, describe, expect, test } from "bun:test";
import { installHost, uninstallHost } from "../../src/js/host.js";
import { mount, useLocale, useTheme } from "../../src/js/contexts.js";
import { jsx } from "../../src/js/jsx-runtime.js";
import { createSignal } from "../../src/js/signals.js";
import { createFakeHost } from "./fake-host.js";

const env = (overrides = {}) => ({
  theme: { colorScheme: "light" },
  locale: { identifier: "en-US", languageCode: "en", textDirection: "ltr" },
  ...overrides,
});

beforeEach(() => {
  uninstallHost();
});

describe("environment contexts", () => {
  test("resolve through component boundaries", () => {
    installHost(createFakeHost(env()));
    const Inner = () => {
      const theme = useTheme();
      return jsx("Text", { children: theme().colorScheme });
    };
    const Outer = () => jsx(Inner, {});
    const { handle, dispose } = mount(() => jsx(Outer, {}));
    expect(handle.children).toEqual(["light"]);
    dispose();
  });

  test("throw outside a mounted tree", () => {
    installHost(createFakeHost(env()));
    expect(() => useTheme()).toThrow(/useTheme/);
  });

  test("track host-owned reactive values", () => {
    const listeners = new Set();
    let current = { colorScheme: "light" };
    const themeSource = {
      read: () => current,
      subscribe(callback) {
        listeners.add(callback);
        return () => listeners.delete(callback);
      },
    };
    installHost(createFakeHost(env({ theme: themeSource })));
    let theme;
    const { dispose } = mount(() => {
      theme = useTheme();
      return jsx("Text", {});
    });
    expect(theme().colorScheme).toBe("light");
    current = { colorScheme: "dark" };
    for (const listener of listeners) {
      listener(current);
    }
    expect(theme().colorScheme).toBe("dark");
    dispose();
    for (const listener of listeners) {
      listener(current);
    }
    expect(listeners.size).toBe(0);
  });

  test("accept signals and constants alike", () => {
    const locale = createSignal({ identifier: "fr", languageCode: "fr", textDirection: "ltr" });
    installHost(createFakeHost(env({ locale })));
    const { dispose } = mount(() => {
      const l = useLocale();
      const theme = useTheme();
      expect(l().identifier).toBe("fr");
      expect(theme().colorScheme).toBe("light");
      return jsx("Text", {});
    });
    dispose();
  });
});

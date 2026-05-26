// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";

const mocks = vi.hoisted(() => {
  return {
    constructor: vi.fn(),
    on: vi.fn(),
    setCenter: vi.fn(),
    addSource: vi.fn(),
    addLayer: vi.fn(),
    getSource: vi.fn(),
    setData: vi.fn(),
    remove: vi.fn(),
    handlers: {} as Record<string, (event: unknown) => void>,
  };
});

vi.mock("maplibre-gl/dist/maplibre-gl.css", () => ({}));

vi.mock("maplibre-gl", () => {
  class MapMock {
    constructor(opts: unknown) {
      mocks.constructor(opts);
    }
    on(event: string, cb: (event: unknown) => void) {
      mocks.on(event);
      mocks.handlers[event] = cb;
    }
    setCenter(coords: unknown) {
      mocks.setCenter(coords);
    }
    addSource(id: string, source: unknown) {
      mocks.addSource(id, source);
    }
    addLayer(layer: unknown) {
      mocks.addLayer(layer);
    }
    getSource(id: string) {
      mocks.getSource(id);
      return { setData: mocks.setData };
    }
    remove() {
      mocks.remove();
    }
  }
  return { default: { Map: MapMock } };
});

import MapPicker from "../components/MapPicker";

beforeEach(() => {
  mocks.constructor.mockClear();
  mocks.on.mockClear();
  mocks.setCenter.mockClear();
  mocks.addSource.mockClear();
  mocks.addLayer.mockClear();
  mocks.getSource.mockClear();
  mocks.setData.mockClear();
  mocks.remove.mockClear();
  for (const key of Object.keys(mocks.handlers)) {
    delete mocks.handlers[key];
  }
});

afterEach(() => {
  cleanup();
});

describe("MapPicker", () => {
  it("mounts and renders a slider initialized to the provided radius", () => {
    const onChange = vi.fn();
    const { container } = render(() => (
      <MapPicker center={[48.85, 2.35]} radiusKm={60} onChange={onChange} />
    ));
    expect(mocks.constructor).toHaveBeenCalledTimes(1);
    const slider = container.querySelector<HTMLInputElement>(
      'input[type="range"]',
    );
    expect(slider).not.toBeNull();
    expect(slider?.value).toBe("60");
  });

  it("fires onChange with the new radius and the same center when the slider moves", () => {
    const onChange = vi.fn();
    const { container } = render(() => (
      <MapPicker center={[48.85, 2.35]} radiusKm={60} onChange={onChange} />
    ));
    const slider = container.querySelector<HTMLInputElement>(
      'input[type="range"]',
    );
    expect(slider).not.toBeNull();
    if (!slider) throw new Error("slider not rendered");
    slider.value = "100";
    slider.dispatchEvent(new Event("input", { bubbles: true }));
    expect(onChange).toHaveBeenCalledWith([48.85, 2.35], 100);
  });

  it("flips LngLat to [lat, lng] in the registered map click handler", () => {
    const onChange = vi.fn();
    render(() => (
      <MapPicker center={[48.85, 2.35]} radiusKm={60} onChange={onChange} />
    ));
    const clickHandler = mocks.handlers["click"];
    expect(clickHandler).toBeDefined();
    clickHandler({ lngLat: { lat: 1.1, lng: 2.2 } });
    expect(onChange).toHaveBeenCalledWith([1.1, 2.2], 60);
  });

  it("calls map.remove() when the component unmounts", () => {
    const onChange = vi.fn();
    const { unmount } = render(() => (
      <MapPicker center={[48.85, 2.35]} radiusKm={60} onChange={onChange} />
    ));
    expect(mocks.remove).not.toHaveBeenCalled();
    unmount();
    expect(mocks.remove).toHaveBeenCalledTimes(1);
  });
});

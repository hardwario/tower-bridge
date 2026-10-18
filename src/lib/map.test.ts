import { describe, expect, it } from "vitest";
import { clampY, decode, mercator, scaleOf } from "./map";

describe("map", () => {
  it("projects to Web Mercator world units", () => {
    expect(mercator(0, 0)).toEqual([0.5, 0.5]);
    expect(mercator(-180, 0)[0]).toBe(0);
    const [x, y] = mercator(15.056328, 50.771168);
    // Liberec is in tile 69/42 at zoom 7 on web maps.
    expect(Math.floor(x * 128)).toBe(69);
    expect(Math.floor(y * 128)).toBe(42);
    // The poles are cut off as on web maps.
    expect(mercator(0, 90)[1]).toBeCloseTo(0, 6);
    expect(scaleOf(0)).toBe(256);
  });

  it("decodes arcs, borders and places", () => {
    // Two squares sharing the arc 0 (their border), one degree per unit.
    const data = decode({
      transform: { scale: [1, 1], translate: [0, 0] },
      arcs: [
        [
          [1, 0],
          [0, 1],
        ],
        [
          [1, 1],
          [-1, 0],
          [0, -1],
          [1, 0],
        ],
        [
          [1, 0],
          [1, 0],
          [0, 1],
          [-1, 0],
        ],
      ],
      objects: {
        countries: {
          type: "GeometryCollection",
          geometries: [
            { type: "Polygon", arcs: [[0, 1]] },
            { type: "Polygon", arcs: [[2, ~0]] },
          ],
        },
        rivers: {
          type: "GeometryCollection",
          geometries: [{ type: "MultiLineString", arcs: [[1]], properties: { z: 6 } }],
        },
        places: {
          type: "GeometryCollection",
          geometries: [{ type: "Point", coordinates: [1, 1], properties: { z: 4.7, n: "Prague", cs: "Praha" } }],
        },
      },
    });
    // Arc 1 starts at the quantized point (1, 1) and runs on by deltas.
    const first = mercator(1, 1);
    expect(data.arcs[1][0]).toBeCloseTo(first[0], 6);
    expect(data.arcs[1][1]).toBeCloseTo(first[1], 6);
    expect(data.land).toHaveLength(2);
    expect(data.borders.map((b) => b.parts)).toEqual([[[0]]]);
    expect(data.rivers[0].minZoom).toBe(6);
    expect(data.places[0]).toMatchObject({ minZoom: 4.7, names: { n: "Prague", cs: "Praha" } });
    // The box of the first square covers longitudes 0 to 1.
    const box = data.land[0].box;
    expect(box[0]).toBeCloseTo(mercator(0, 0)[0], 6);
    expect(box[2]).toBeCloseTo(mercator(1, 0)[0], 6);
  });

  it("continues outlines across the 180th meridian", () => {
    // An island from 170° E to 170° W, its outline starting on either side.
    for (const start of [170, -170]) {
      const other = start === 170 ? -170 : 170;
      const ring: [number, number][] = [
        [start, 60],
        [other - start, 0],
        [0, 10],
        [start - other, 0],
        [0, -10],
      ];
      const data = decode({
        transform: { scale: [1, 1], translate: [0, 0] },
        arcs: [ring],
        objects: { countries: { type: "GeometryCollection", geometries: [{ type: "Polygon", arcs: [[0]] }] } },
      });
      const [x0, , x1] = data.land[0].box;
      // 20° wide where it is drawn, not the whole world wide.
      expect(x1 - x0).toBeCloseTo(20 / 360, 6);
      expect(x0).toBeCloseTo(mercator(170, 0)[0], 6);
    }
  });

  it("keeps the view between the top and the bottom of the map", () => {
    // 240 px at zoom 2: the view is 240 / 1024 world units high.
    const half = 120 / 1024;
    expect(clampY(0.999, 2, 240)).toBeCloseTo(1 - half, 9);
    expect(clampY(0.0, 2, 240)).toBeCloseTo(half, 9);
    expect(clampY(0.4, 2, 240)).toBe(0.4);
    // Zoomed out so far that the whole map is lower than the view: centred.
    expect(clampY(0.9, 0, 600)).toBe(0.5);
  });
});

import { describe, expect, it } from "vitest";
import { decode, mercator, scaleOf } from "./map";

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
});

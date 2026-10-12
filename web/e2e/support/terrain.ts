import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
/** Explicit synthetic test terrain; never uses or replaces the development dataset. */
export async function installFixtureTerrain(directory: string) {
  const root = path.join(directory, "terrain"); await mkdir(root);
  const data = Buffer.alloc(1201 * 1201 * 2), sha256 = createHash("sha256").update(data).digest("hex");
  const tiles = [];
  for (const latitude of [37, 38]) for (const longitude of [-78, -77, -76]) {
    const file = `N${latitude}W${Math.abs(longitude).toString().padStart(3, "0")}.hgt`;
    await writeFile(path.join(root, file), data);
    tiles.push({ file, latitude, longitude, samples: 1201, sha256 });
  }
  await writeFile(path.join(root, "manifest.json"), JSON.stringify({ version: 1, source: "Synthetic flat terrain for isolated browser tests; not acceptance evidence for real SRTM", vertical_datum: "EGM96", tiles }));
  return root;
}

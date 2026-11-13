// Azure's retail prices API names a region `eastus`; the VM price files (and Studio's region menu) use
// `us-east`. vmName joins the two by trying the readable spellings of the API name against the VM region
// list; a region with no match gives null and the caller reports and leaves it out (nothing is guessed).
// Shared by gen-azure-sql.mjs and gen-egress.mjs (gen-storage-prices.mjs keeps its own copy).
import { readdirSync } from "node:fs";

const vmRegions = new Set(
  readdirSync(new URL("../crates/kaanal-core/data/prices/azure/", import.meta.url)).map((f) => f.replace(/.csv$/, "")),
);
const DIRS = ["southcentral", "northcentral", "westcentral", "southeast", "northeast", "southwest", "northwest", "central", "east", "west", "north", "south"];
const ALIAS = { asia: ["asia-pacific"], southafrica: ["south-africa"], newzealand: ["new-zealand"] };
const hyphen = (d) => d.replace(/^(south|north|west)(central)$/, "$1-$2");

export function vmName(arm) {
  // a trailing number is part of the VM name too (eastus2 is us-east-2)
  const num = /^(.*?)([0-9]+)$/.exec(arm);
  if (num) {
    const base = vmName(num[1]);
    return base && vmRegions.has(`${base}-${num[2]}`) ? `${base}-${num[2]}` : null;
  }
  for (const d of DIRS) {
    for (const geo of [arm.startsWith(d) ? arm.slice(d.length) : null, arm.endsWith(d) ? arm.slice(0, -d.length) : null]) {
      if (!geo) continue;
      for (const g of ALIAS[geo] ?? [geo]) {
        for (const c of [`${g}-${hyphen(d)}`, `${hyphen(d)}-${g}`]) if (vmRegions.has(c)) return c;
      }
    }
  }
  return null;
}

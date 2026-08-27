// Temporary: verify lens data matches between get_lens_list and file rows.
const { DatabaseSync } = require("node:sqlite");
const db = new DatabaseSync(process.env.APPDATA + "\\com.tiol.desktop\\db.sqlite", { readOnly: true });
const lenses = db
  .prepare("SELECT DISTINCT lens FROM files WHERE lens IS NOT NULL AND lens != '' ORDER BY lens")
  .all();
console.log("lens list:", JSON.stringify(lenses));
for (const l of lenses) {
  // safe: lens strings come from our own EXIF reader, no quotes inside
  const n = db.prepare("SELECT COUNT(*) n FROM files WHERE lens = '" + l.lens.replace(/'/g, "''") + "'").get();
  console.log(JSON.stringify(l.lens), "->", n.n);
}

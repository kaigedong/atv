'use strict';
/* Produce a portable HTML entry that works without sibling script/style files. */
const fs = require('node:fs/promises');
const path = require('node:path');

async function main() {
  const destination = process.argv[2];
  if (!destination) throw new Error('Usage: node build-standalone.cjs /path/to/sidecar-slalom.html');
  const output = path.resolve(destination);
  if (output === path.join(__dirname, 'index.html')) throw new Error('Use a separate output path; do not overwrite the source entry.');
  const [html, css, model, app] = await Promise.all(['index.html', 'style.css', 'model.js', 'app.js'].map(name => fs.readFile(path.join(__dirname, name), 'utf8')));
  const substitutions = [
    ['<link rel="stylesheet" href="style.css">', `<style>\n${css.replace(/<\/style/gi, '<\\/style')}\n</style>`],
    ['<script src="model.js"></script>', `<script>\n${model.replace(/<\/script/gi, '<\\/script')}\n</script>`],
    ['<script src="app.js"></script>', `<script>\n${app.replace(/<\/script/gi, '<\\/script')}\n</script>`]
  ];
  let document = html;
  for (const [marker, replacement] of substitutions) {
    if (document.split(marker).length !== 2) throw new Error(`Expected one asset marker: ${marker}`);
    // Function replacement preserves literal $ tokens in embedded JavaScript.
    document = document.replace(marker, () => replacement);
  }
  await fs.mkdir(path.dirname(output), { recursive: true });
  await fs.writeFile(output, document, 'utf8');
  console.log(`Standalone HTML created: ${path.basename(output)} (${Buffer.byteLength(document)} bytes)`);
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });

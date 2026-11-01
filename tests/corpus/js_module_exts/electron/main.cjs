const { spawnHelper } = require("./spawn.cjs");

function createWindow() {
  return spawnHelper("viewer");
}

function main() {
  return createWindow();
}

module.exports = { main };

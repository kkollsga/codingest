# Entry point: source() chain A -> B -> C runs through sub/helpers.R.
source("utils.R")
source("sub/helpers.R")
source("legacy.r")

# External package: resolves to nothing, produces no edge.
library(stats)
# BAIT: a package name colliding with the local tools.R module. A package
# reference is namespace-shaped and must produce neither a File->File edge
# nor an edge onto the root-prefixed local module.
library(tools)
require(methods)

# Non-literal source() argument: extracts nothing.
source(paste0(script_dir, "/dynamic.R"))

# Orchestrates the pipeline.
main <- function(n) {
  values <- run_all(n)
  add_one(values)
}

run_all = function(n) {
  if (n > 0) {
    mul(n, 2)
  } else {
    0
  }
}

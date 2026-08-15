# Adds one to its argument.
add_one <- function(x) {
  x + 1
}

# The `=` assignment shape; calls add_one same-file.
mul = function(a, b = 2) {
  add_one(a) * b
}

# R 4.1 backslash-lambda shape.
double_it <- \(x) x * 2

# Hidden by convention (leading dot) -> private visibility.
.internal_scale <- function(x, factor = 10) {
  x * factor
}

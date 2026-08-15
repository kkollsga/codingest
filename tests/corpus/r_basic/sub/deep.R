# C of the source chain; self-recursive for is_recursive coverage.
deep_count <- function(n) {
  if (n > 0) {
    deep_count(n - 1)
  } else {
    0
  }
}

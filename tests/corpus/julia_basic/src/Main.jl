# Entry point: stitches the package together with include() and pulls in a
# local module plus an external package whose name collides with a file bait.
module JuliaBasic

include("geometry.jl")
include("util.jl")

# Local module reference — namespace-shaped, resolves to no file edge.
using .Geometry

# BAIT: an external package named exactly like src/Downloads.jl in this
# corpus. A `using` is a module reference, never a file path, so no
# File->File edge may appear from Main.jl to Downloads.jl.
using Downloads

export run_report, MAX_RADIUS

const MAX_RADIUS = 10.0

"""
Format a report line for a radius.
"""
function run_report(r)
    label = describe(r)
    return label
end

describe(r) = r > MAX_RADIUS ? "large" : "small"

end # module

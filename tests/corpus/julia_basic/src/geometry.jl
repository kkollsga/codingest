module Geometry

# Second hop of the include chain: Main.jl -> geometry.jl -> shapes/circle.jl.
include("shapes/circle.jl")

export area, scaled, Shape

abstract type Shape end

"""
Area of a circle.
"""
function area(c::Circle)
    return pi * c.radius^2
end

# Multiple dispatch: same name, different signature — the builder must keep
# both methods as distinct nodes.
function area(c::Circle, scale::Float64)
    base = area(c)
    if scale < 0.0
        return 0.0
    end
    return scale * base
end

# Short-form method definition.
scaled(c::Circle, k) = k * area(c)

end # module

"""
A circle with a label, subtyping the local Shape hierarchy.
"""
struct Circle <: Shape
    radius::Float64
    label::String
end

mutable struct Counter
    count::Int
end

circumference(c::Circle) = 2 * pi * c.radius

function bump!(counter::Counter)
    counter.count += 1
    return counter.count
end

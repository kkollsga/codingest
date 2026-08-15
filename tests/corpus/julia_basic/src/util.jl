# Top-level helpers outside any module block.

const GREETING = "hello"

function describe_size(x)
    if x > 100
        return "big"
    elseif x > 10
        return "medium"
    end
    for _ in 1:3
        x = shrink(x)
    end
    return "small"
end

shrink(x) = x / 2

function countdown(n)
    if n <= 0
        return 0
    end
    return countdown(n - 1)
end

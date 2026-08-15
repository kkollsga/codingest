# BAIT FILE: shares its name with the external `Downloads` package that
# Main.jl `using`s. Never included by any file — if a File->File IMPORTS edge
# ever points here, a namespace reference was misread as a file path.
download_stub(url) = url

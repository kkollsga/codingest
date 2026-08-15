#include "local.h"
#include "util/helper.h"
#include <vector>
// Angle-collision pin: <local.h> names a REAL project file, but an angle
// include is a system lookup — it must form no File→File edge. Before the
// quoted/angle split this exact shape manufactured one.
#include <local.h>

int main() {
    std::vector<int> v;
    v.push_back(local_value());
    v.push_back(util_helper());
    return static_cast<int>(v.size());
}

#include "local.h"
#include "util/helper.h"
#include <vector>

int main() {
    std::vector<int> v;
    v.push_back(local_value());
    v.push_back(util_helper());
    return static_cast<int>(v.size());
}

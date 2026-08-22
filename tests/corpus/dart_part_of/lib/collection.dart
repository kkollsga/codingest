// Parent library, split across two `part` files that live one directory
// deeper. All three files must land in ONE module.
library collection;

part 'src/a.dart';
part 'src/b.dart';

int seed() {
  return 7;
}

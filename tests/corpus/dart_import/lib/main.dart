import 'package:dart_import/a/x.dart';
import 'package:dart_import/b/x.dart' as bx;
import 'a/x.dart' show aValue;

int combine() {
  return aValue() + bx.bValue();
}

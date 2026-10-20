int generic_target(void) { return 1; }
int generic_probe(void) {
    return _Generic(1, int: generic_target(), default: 0);
}
int after_generic_probe(void) { return generic_target(); }

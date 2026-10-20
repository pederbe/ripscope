int (*pointer_callback)(int);
int dereferenced_probe(void) { return (*pointer_callback)(1); }
int direct_probe(void) { return pointer_callback(1); }
int pointer_neighbor(void) { return direct_probe(); }

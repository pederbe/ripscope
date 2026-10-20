char *greeting = "hello(world); int fake;";
char quote = '\'';
char *escaped = "a \" b(c) \\";
char *continued = "part one \
part two(call)";
int after_literal(void) { return strlen("x(y)"); }
char *unterminated = "open
int recovered(void) { return 1; }

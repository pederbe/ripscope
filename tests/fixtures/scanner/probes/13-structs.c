struct tagged;
struct tagged *pointer;
struct tagged { int member; };
struct { int anonymous_member; } anonymous_var;
struct outer { struct inner { int deep; } nested; int shallow; };
enum { UNNAMED_A, UNNAMED_B };
union /* comment */ commented { int c; };
struct later
{
	int on_next_line;
};

typedef _Bool boolean_alias;
_Bool boolean_object;
double _Complex complex_object;
int *restrict restricted_object;
struct flexible_tag {
    unsigned bit_member : 3;
    int length_member;
    char flexible_member[];
};
struct pair_tag { int x_member; int y_member; };
struct pair_tag designated_object = {.x_member = 1, .y_member = 2};
inline int inline_function(int n) { return n; }
int c99_leaf(int n) { return n; }
int vla_function(int n, int values[static 2]) {
    int local_array[n];
    local_array[0] = values[0];
    return c99_leaf(local_array[0]);
}
int compound_call(void) {
    struct pair_tag p = (struct pair_tag){.x_member = c99_leaf(1), .y_member = 2};
    return c99_leaf(p.x_member);
}
int after_c99(void) { return c99_leaf(0); }

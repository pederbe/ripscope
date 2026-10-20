int ordinary_object;
static int internal_object;
const volatile unsigned long qualified_object = 1;
int *object_pointer;
int object_array[4];
int (*array_pointer)[4];
int (*callback_object)(int);
int (*callback_array[2])(int);
int (*(*nested_callback)(void))(int);
typedef unsigned long count_alias;
typedef int function_alias(int prototype_parameter);
typedef int (*callback_alias)(int callback_parameter);
typedef int array_alias[4];
struct record_tag {
    int ordinary_member;
    int (*callback_member)(int member_parameter);
    int (*array_member)[4];
};
union choice_tag { int choice_member; double real_member; };
enum color_tag { red_value, blue_value };
extern int external_object;
int prototype_only(int prototype_name);
int leaf(int value) { return value; }
int direct_call(void) { return leaf(1); }
int repeated_call(void) { return leaf(1) + leaf(2); }
int recursive_call(int n) { return n ? recursive_call(n - 1) : 0; }
int callback_call(int (*parameter_callback)(int)) {
    return parameter_callback(1);
}
int dereferenced_call(void) { return (*callback_object)(1); }
int member_call(struct record_tag *p) { return p->callback_member(1); }
int old_style(n)
int n;
{ return leaf(n); }
int (*return_callback(void))(int) { return leaf; }
int following_function(void) { return leaf(3); }
int keyword_controls(void) {
    int n = 0;
    if (n) n++;
    while (n) n--;
    for (; n; n--) n++;
    switch (n) { case 0: break; default: break; }
    return (int)sizeof(n);
}

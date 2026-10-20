int (*pointer_callback)(int);
int dereferenced_probe(void) { return (*pointer_callback)(1); }
int direct_probe(void) { return pointer_callback(1); }
int pointer_neighbor(void) { return direct_probe(); }
struct operations { int (*member)(int); };
void caller(struct operations *ops, int (*parameter)(int)) {
    direct_probe(); parameter(1); ops->member(2);
    (*parameter)(3); (*parameter)(4); parameter(5);
    int (*local)(int);
    int *(*pointer_return)(int);
    T (*typed_local)(Arg);
    (void)*local; (void)(*local);
    int (*array[2])(int);
    direct_probe();
    (
        * /* prefix */ parameter
    ) /* suffix */
    (6);
    neighbor((*parameter)(7));
    if (flag) (*parameter)(8);
    else (*parameter)(9);
    (*parameter) + (1);
    (*parameter)[1];
    (*parameter + other)(1);
    ("(*fake)(1)");
    /* (*comment_only)(1); */
}
#define INVOKE(p) (*p)(1)
void declaration_lists(void) {
    int (*first)(int), (*second)(int);
    T (*third)(Arg), (*fourth)(Arg);
    int (*initialized)(int) = (*factory)(1), (*last)(int);
    (void)0, (*expression_callback)(1);
    factory_result()(*not_called)(1);
    int \
        (*spliced_declaration)(int);
}
void declaration_controls(void) {
    int normal, (*local_callback)(int);
    int (*const first)(int), (*qualified_second)(int);
    int (*volatile vfirst)(int), (*volatile_second)(int);
    int ((*nested_decl)(int));
    int (*array[2])(int), (*array_list_cb)(int);
    int variable = (*factory)((*nested_value)(1)), (*last_value)(int);
    int (*good)(int), (*(*returning)(void))(int);
    (void)(*discarded)(1);
    (int)(*casted)(1);
    if (flag) (void)(*controlled)(1);
    (*outer)((*nested)(2));
}
void conditional_expression(void) { return flag ? (*selected)(1) : (*other_selected)(2); }
void directives(void) {
#if FLAG
    (*arm_one)(1);
#else
    (*arm_two)(2);
#endif
    (*after_conditional)(3);
#define LOCAL 1
    (*after_define)(4);
    int
#if TYPE
    (*decl_one)(int);
#else
    (*decl_two)(int);
#endif
}
void expression_controls(void) {
    int scalar = plain_call(1, (*initializer_cb)(2));
    int (*callback)(int) = (*factory)(1, (*nested_initializer_cb)(2));
    int nested_array[indices[0] + (*nested_bound_cb)(1)];
    if ((int)(*condition_callback)(1)) (*body_callback)(2);
    while ((int)(*loop_condition)(1)) (*loop_body)(2);
}
void designated_initializers(void) {
    int array[2] = { [0] = (*element_cb)(1) }, (*after_array)(int);
    struct S object = { .x = (*field_cb)(1), .y = (*other_field_cb)(2) }, (*after_struct)(int);
}
void aggregate_declarations(void) {
    struct { int x; } object, (*after_anonymous)(int);
    struct { int x; } initialized = { .x = (*aggregate_cb)(1) }, (*after_anonymous_init)(int);
    struct Named { int x; } named, (*after_named)(int);
    union { int x; } combined, (*after_union)(int);
    enum { VALUE } enumerated, (*after_enum)(int);
    register int ordinary, (*after_register)(int);
    auto int local, (*after_auto)(int);
    _Alignas(16) int aligned, (*after_alignas)(int);
    _Thread_local static int thread_local, (*after_thread_local)(int);
}
void optional_control(void) {
#if FLAG
    if (flag)
#endif
    (*optional_cb)(1);
}
void optional_else(void) {
#if FLAG
    if (flag)
#else
#endif
    (*optional_cb)(2);
}
void for_declaration(void) {
    for (int i = 0, (*for_pointer)(int); i < (*for_condition)(1); i += (*for_step)(1)) {
        (*for_body)(i);
    }
}
typedef int Count;
void for_typedef_declaration(void) {
    for (Count i = 0, (*typedef_pointer)(int); (*typed_condition)(i); (*typed_increment)(i)) {
        (*typed_body)(i);
    }
}

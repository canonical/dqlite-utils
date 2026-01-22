use std::iter::Peekable;
use std::str::Chars;

use anyhow::anyhow;
use owo_colors::Style;
use rusqlite::types::ValueRef;
use sqlparser::ast::{
    ColumnOption, Expr, GranteesType, Ident, ObjectNamePart, Statement, VacuumStatement,
};
use sqlparser::dialect::{Dialect, Precedence, SQLiteDialect};
use sqlparser::keywords::Keyword;
use sqlparser::parser::{Parser, ParserError};

use crate::utils::TerminalStylizeExt;
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct SqlCommand {
    raw: String,
}

// Override upstream SQLiteDialect as it's missing some features
#[derive(Debug)]
struct DqliteDialect(SQLiteDialect);

impl DqliteDialect {
    fn new() -> Self {
        DqliteDialect(SQLiteDialect {})
    }

    fn parse_vacuum(&self, parser: &mut Parser) -> Result<Statement, ParserError> {
        parser.parse_optional_ident()?;
        if parser.parse_keyword(Keyword::INTO) {
            parser.parse_literal_string()?;
        }
        return Ok(Statement::Vacuum(VacuumStatement {
            full: false,
            sort_only: false,
            delete_only: false,
            reindex: false,
            recluster: false,
            table_name: None,
            threshold: None,
            boost: false,
        }));
    }
}

impl Dialect for DqliteDialect {
    fn parse_statement(&self, parser: &mut Parser) -> Option<Result<Statement, ParserError>> {
        if parser.parse_keyword(Keyword::VACUUM) {
            Some(self.parse_vacuum(parser))
        } else {
            self.0.parse_statement(parser)
        }
    }

    // Below are just delegations to SQLiteDialect
    fn dialect(&self) -> std::any::TypeId {
        self.0.dialect()
    }

    fn is_identifier_start(&self, ch: char) -> bool {
        self.0.is_identifier_start(ch)
    }

    fn is_identifier_part(&self, ch: char) -> bool {
        self.0.is_identifier_part(ch)
    }

    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        self.0.is_delimited_identifier_start(ch)
    }

    fn is_nested_delimited_identifier_start(&self, ch: char) -> bool {
        self.0.is_nested_delimited_identifier_start(ch)
    }

    fn peek_nested_delimited_identifier_quotes(
        &self,
        chars: Peekable<Chars<'_>>,
    ) -> Option<(char, Option<char>)> {
        self.0.peek_nested_delimited_identifier_quotes(chars)
    }

    fn identifier_quote_style(&self, identifier: &str) -> Option<char> {
        self.0.identifier_quote_style(identifier)
    }

    fn is_custom_operator_part(&self, _ch: char) -> bool {
        self.0.is_custom_operator_part(_ch)
    }

    fn supports_string_literal_backslash_escape(&self) -> bool {
        self.0.supports_string_literal_backslash_escape()
    }

    fn ignores_wildcard_escapes(&self) -> bool {
        self.0.ignores_wildcard_escapes()
    }

    fn supports_unicode_string_literal(&self) -> bool {
        self.0.supports_unicode_string_literal()
    }

    fn supports_filter_during_aggregation(&self) -> bool {
        self.0.supports_filter_during_aggregation()
    }

    fn supports_window_clause_named_window_reference(&self) -> bool {
        self.0.supports_window_clause_named_window_reference()
    }

    fn supports_within_after_array_aggregation(&self) -> bool {
        self.0.supports_within_after_array_aggregation()
    }

    fn supports_group_by_expr(&self) -> bool {
        self.0.supports_group_by_expr()
    }

    fn supports_group_by_with_modifier(&self) -> bool {
        self.0.supports_group_by_with_modifier()
    }

    fn supports_left_associative_joins_without_parens(&self) -> bool {
        self.0.supports_left_associative_joins_without_parens()
    }

    fn supports_outer_join_operator(&self) -> bool {
        self.0.supports_outer_join_operator()
    }

    fn supports_cross_join_constraint(&self) -> bool {
        self.0.supports_cross_join_constraint()
    }

    fn supports_connect_by(&self) -> bool {
        self.0.supports_connect_by()
    }

    fn supports_execute_immediate(&self) -> bool {
        self.0.supports_execute_immediate()
    }

    fn supports_match_recognize(&self) -> bool {
        self.0.supports_match_recognize()
    }

    fn supports_in_empty_list(&self) -> bool {
        self.0.supports_in_empty_list()
    }

    fn supports_start_transaction_modifier(&self) -> bool {
        self.0.supports_start_transaction_modifier()
    }

    fn supports_end_transaction_modifier(&self) -> bool {
        self.0.supports_end_transaction_modifier()
    }

    fn supports_named_fn_args_with_eq_operator(&self) -> bool {
        self.0.supports_named_fn_args_with_eq_operator()
    }

    fn supports_named_fn_args_with_colon_operator(&self) -> bool {
        self.0.supports_named_fn_args_with_colon_operator()
    }

    fn supports_named_fn_args_with_assignment_operator(&self) -> bool {
        self.0.supports_named_fn_args_with_assignment_operator()
    }

    fn supports_named_fn_args_with_rarrow_operator(&self) -> bool {
        self.0.supports_named_fn_args_with_rarrow_operator()
    }

    fn supports_named_fn_args_with_expr_name(&self) -> bool {
        self.0.supports_named_fn_args_with_expr_name()
    }

    fn supports_numeric_prefix(&self) -> bool {
        self.0.supports_numeric_prefix()
    }

    fn supports_numeric_literal_underscores(&self) -> bool {
        self.0.supports_numeric_literal_underscores()
    }

    fn supports_window_function_null_treatment_arg(&self) -> bool {
        self.0.supports_window_function_null_treatment_arg()
    }

    fn supports_dictionary_syntax(&self) -> bool {
        self.0.supports_dictionary_syntax()
    }

    fn support_map_literal_syntax(&self) -> bool {
        self.0.support_map_literal_syntax()
    }

    fn supports_lambda_functions(&self) -> bool {
        self.0.supports_lambda_functions()
    }

    fn supports_parenthesized_set_variables(&self) -> bool {
        self.0.supports_parenthesized_set_variables()
    }

    fn supports_comma_separated_set_assignments(&self) -> bool {
        self.0.supports_comma_separated_set_assignments()
    }

    fn supports_select_wildcard_except(&self) -> bool {
        self.0.supports_select_wildcard_except()
    }

    fn convert_type_before_value(&self) -> bool {
        self.0.convert_type_before_value()
    }

    fn supports_triple_quoted_string(&self) -> bool {
        self.0.supports_triple_quoted_string()
    }

    fn parse_prefix(&self, parser: &mut Parser) -> Option<Result<Expr, ParserError>> {
        self.0.parse_prefix(parser)
    }

    fn supports_trailing_commas(&self) -> bool {
        self.0.supports_trailing_commas()
    }

    fn supports_limit_comma(&self) -> bool {
        self.0.supports_limit_comma()
    }

    fn supports_string_literal_concatenation(&self) -> bool {
        self.0.supports_string_literal_concatenation()
    }

    fn supports_projection_trailing_commas(&self) -> bool {
        self.0.supports_projection_trailing_commas()
    }

    fn supports_from_trailing_commas(&self) -> bool {
        self.0.supports_from_trailing_commas()
    }

    fn supports_column_definition_trailing_commas(&self) -> bool {
        self.0.supports_column_definition_trailing_commas()
    }

    fn supports_object_name_double_dot_notation(&self) -> bool {
        self.0.supports_object_name_double_dot_notation()
    }

    fn supports_struct_literal(&self) -> bool {
        self.0.supports_struct_literal()
    }

    fn supports_empty_projections(&self) -> bool {
        self.0.supports_empty_projections()
    }

    fn supports_select_expr_star(&self) -> bool {
        self.0.supports_select_expr_star()
    }

    fn supports_from_first_select(&self) -> bool {
        self.0.supports_from_first_select()
    }

    fn supports_pipe_operator(&self) -> bool {
        self.0.supports_pipe_operator()
    }

    fn supports_user_host_grantee(&self) -> bool {
        self.0.supports_user_host_grantee()
    }

    fn supports_match_against(&self) -> bool {
        self.0.supports_match_against()
    }

    fn supports_select_wildcard_exclude(&self) -> bool {
        self.0.supports_select_wildcard_exclude()
    }

    fn supports_select_exclude(&self) -> bool {
        self.0.supports_select_exclude()
    }

    fn supports_create_table_multi_schema_info_sources(&self) -> bool {
        self.0.supports_create_table_multi_schema_info_sources()
    }

    fn parse_infix(
        &self,
        parser: &mut Parser,
        expr: &Expr,
        precedence: u8,
    ) -> Option<Result<Expr, ParserError>> {
        self.0.parse_infix(parser, expr, precedence)
    }

    fn get_next_precedence(&self, parser: &Parser) -> Option<Result<u8, ParserError>> {
        self.0.get_next_precedence(parser)
    }

    fn get_next_precedence_default(&self, parser: &Parser) -> Result<u8, ParserError> {
        self.0.get_next_precedence_default(parser)
    }

    fn parse_column_option(
        &self,
        parser: &mut Parser,
    ) -> Result<Option<Result<Option<ColumnOption>, ParserError>>, ParserError> {
        self.0.parse_column_option(parser)
    }

    fn prec_value(&self, prec: Precedence) -> u8 {
        self.0.prec_value(prec)
    }

    fn prec_unknown(&self) -> u8 {
        self.0.prec_unknown()
    }

    fn describe_requires_table_keyword(&self) -> bool {
        self.0.describe_requires_table_keyword()
    }

    fn allow_extract_custom(&self) -> bool {
        self.0.allow_extract_custom()
    }

    fn allow_extract_single_quotes(&self) -> bool {
        self.0.allow_extract_single_quotes()
    }

    fn supports_dollar_placeholder(&self) -> bool {
        self.0.supports_dollar_placeholder()
    }

    fn supports_create_index_with_clause(&self) -> bool {
        self.0.supports_create_index_with_clause()
    }

    fn require_interval_qualifier(&self) -> bool {
        self.0.require_interval_qualifier()
    }

    fn supports_explain_with_utility_options(&self) -> bool {
        self.0.supports_explain_with_utility_options()
    }

    fn supports_asc_desc_in_column_definition(&self) -> bool {
        self.0.supports_asc_desc_in_column_definition()
    }

    fn supports_factorial_operator(&self) -> bool {
        self.0.supports_factorial_operator()
    }

    fn supports_nested_comments(&self) -> bool {
        self.0.supports_nested_comments()
    }

    fn supports_eq_alias_assignment(&self) -> bool {
        self.0.supports_eq_alias_assignment()
    }

    fn supports_try_convert(&self) -> bool {
        self.0.supports_try_convert()
    }

    fn supports_bang_not_operator(&self) -> bool {
        self.0.supports_bang_not_operator()
    }

    fn supports_listen_notify(&self) -> bool {
        self.0.supports_listen_notify()
    }

    fn supports_load_data(&self) -> bool {
        self.0.supports_load_data()
    }

    fn supports_load_extension(&self) -> bool {
        self.0.supports_load_extension()
    }

    fn supports_top_before_distinct(&self) -> bool {
        self.0.supports_top_before_distinct()
    }

    fn supports_boolean_literals(&self) -> bool {
        self.0.supports_boolean_literals()
    }

    fn supports_show_like_before_in(&self) -> bool {
        self.0.supports_show_like_before_in()
    }

    fn supports_comment_on(&self) -> bool {
        self.0.supports_comment_on()
    }

    fn supports_create_table_select(&self) -> bool {
        self.0.supports_create_table_select()
    }

    fn supports_partiql(&self) -> bool {
        self.0.supports_partiql()
    }

    fn is_reserved_for_identifier(&self, kw: Keyword) -> bool {
        self.0.is_reserved_for_identifier(kw)
    }

    fn get_reserved_keywords_for_select_item_operator(&self) -> &[Keyword] {
        self.0.get_reserved_keywords_for_select_item_operator()
    }

    fn get_reserved_grantees_types(&self) -> &[GranteesType] {
        self.0.get_reserved_grantees_types()
    }

    fn supports_table_sample_before_alias(&self) -> bool {
        self.0.supports_table_sample_before_alias()
    }

    fn supports_insert_set(&self) -> bool {
        self.0.supports_insert_set()
    }

    fn supports_insert_table_function(&self) -> bool {
        self.0.supports_insert_table_function()
    }

    fn supports_insert_format(&self) -> bool {
        self.0.supports_insert_format()
    }

    fn supports_set_stmt_without_operator(&self) -> bool {
        self.0.supports_set_stmt_without_operator()
    }

    fn is_column_alias(&self, kw: &Keyword, parser: &mut Parser) -> bool {
        self.0.is_column_alias(kw, parser)
    }

    fn is_select_item_alias(&self, explicit: bool, kw: &Keyword, parser: &mut Parser) -> bool {
        self.0.is_select_item_alias(explicit, kw, parser)
    }

    fn is_table_factor(&self, kw: &Keyword, parser: &mut Parser) -> bool {
        self.0.is_table_factor(kw, parser)
    }

    fn is_table_alias(&self, kw: &Keyword, parser: &mut Parser) -> bool {
        self.0.is_table_alias(kw, parser)
    }

    fn is_table_factor_alias(&self, explicit: bool, kw: &Keyword, parser: &mut Parser) -> bool {
        self.0.is_table_factor_alias(explicit, kw, parser)
    }

    fn supports_timestamp_versioning(&self) -> bool {
        self.0.supports_timestamp_versioning()
    }

    fn supports_string_escape_constant(&self) -> bool {
        self.0.supports_string_escape_constant()
    }

    fn supports_table_hints(&self) -> bool {
        self.0.supports_table_hints()
    }

    fn requires_single_line_comment_whitespace(&self) -> bool {
        self.0.requires_single_line_comment_whitespace()
    }

    fn supports_array_typedef_with_brackets(&self) -> bool {
        self.0.supports_array_typedef_with_brackets()
    }

    fn supports_geometric_types(&self) -> bool {
        self.0.supports_geometric_types()
    }

    fn supports_order_by_all(&self) -> bool {
        self.0.supports_order_by_all()
    }

    fn supports_set_names(&self) -> bool {
        self.0.supports_set_names()
    }

    fn supports_space_separated_column_options(&self) -> bool {
        self.0.supports_space_separated_column_options()
    }

    fn supports_alter_column_type_using(&self) -> bool {
        self.0.supports_alter_column_type_using()
    }

    fn supports_comma_separated_drop_column_list(&self) -> bool {
        self.0.supports_comma_separated_drop_column_list()
    }

    fn is_identifier_generating_function_name(
        &self,
        ident: &Ident,
        name_parts: &[ObjectNamePart],
    ) -> bool {
        self.0
            .is_identifier_generating_function_name(ident, name_parts)
    }

    fn supports_notnull_operator(&self) -> bool {
        self.0.supports_notnull_operator()
    }

    fn supports_data_type_signed_suffix(&self) -> bool {
        self.0.supports_data_type_signed_suffix()
    }

    fn supports_interval_options(&self) -> bool {
        self.0.supports_interval_options()
    }

    fn supports_create_table_like_parenthesized(&self) -> bool {
        self.0.supports_create_table_like_parenthesized()
    }

    fn supports_semantic_view_table_factor(&self) -> bool {
        self.0.supports_semantic_view_table_factor()
    }
}

impl SqlCommand {
    pub(crate) fn try_from_raw(raw: &str) -> Result<Self> {
        let dialect = DqliteDialect::new();
        let mut parser = Parser::new(&dialect)
            .with_recursion_limit(100)
            .try_with_sql(raw)?;
        parser.try_parse(|parser| parser.parse_statements())?;

        let raw = raw.to_owned();
        Ok(Self { raw })
    }

    pub(crate) fn run(self, ctx: &Context) -> Result<()> {
        let Self { raw } = self;
        let conn = ctx.shell.connection().ok_or_else(|| {
            anyhow!(
                "sql execution not available in {} shell",
                ctx.shell.kind().name()
            )
        })?;
        let mut stmt = conn.prepare(&raw)?;
        {
            let column_count = stmt.column_count();

            // Print header
            if column_count > 0 {
                for i in 0..column_count {
                    print!("{}  ", stmt.column_name(i)?);
                }
                println!("\n---");
            }

            // Print content
            let mut rows = stmt.query(())?;
            while let Some(row) = rows.next()? {
                for i in 0..column_count {
                    match row.get_ref(i)? {
                        ValueRef::Blob(blob) => print!("<blob:({}B)>  ", blob.len()),
                        ValueRef::Null => print!("NULL  "),
                        ValueRef::Integer(value) => print!("{}  ", value),
                        ValueRef::Real(value) => print!("{}  ", value),
                        ValueRef::Text(text) => {
                            print!("{}  ", String::from_utf8_lossy(text));
                        }
                    }
                }
                println!();
            }
        }

        if !stmt.readonly() {
            const ROWS_AFFECTED_STYLE: Style = Style::new().dimmed();
            println!(
                "{} {}",
                conn.changes().terminal_style(ROWS_AFFECTED_STYLE),
                "rows affected".terminal_style(ROWS_AFFECTED_STYLE)
            );
        }
        Ok(())
    }
}

//! R3 Humanizer - Convert R3 AST to flat list for comparison.
//!
//! Matches Angular's R3AstHumanizer from r3_template_transform_spec.ts.

use oxc_angular_compiler::ast::r3::{
    R3BoundText, R3Content, R3Element, R3Node, R3Reference, R3Template, R3TemplateAttr, R3Text,
    R3Variable, R3Visitor, visit_all,
};

use super::util::{binding_type_to_string, event_type_to_string};
use crate::subsystems::unparser::{normalize_whitespace, unparse_expression_r3};

/// Mode for humanizing R3 AST
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HumanizeMode {
    /// Template transform mode: output node type and name
    /// e.g., [Element, div], [BoundText, {{ a }}]
    TemplateTransform,
    /// Source spans mode: output node type and source spans
    /// e.g., [Element, <div></div>, <div>, </div>], [BoundText, {{a}}]
    SourceSpans,
}

/// Humanizer that converts R3 AST to a flat list for test comparison.
pub struct R3Humanizer<'s> {
    result: Vec<Vec<String>>,
    source: &'s str,
    mode: HumanizeMode,
}

impl<'s> R3Humanizer<'s> {
    fn new(source: &'s str, mode: HumanizeMode) -> Self {
        R3Humanizer { result: Vec::new(), source, mode }
    }

    pub fn humanize_nodes(
        nodes: &[R3Node<'_>],
        source: &str,
        mode: HumanizeMode,
    ) -> Vec<Vec<String>> {
        let mut humanizer = R3Humanizer::new(source, mode);
        visit_all(&mut humanizer, nodes);
        humanizer.result
    }

    fn visit_nodes(&mut self, nodes: &[R3Node<'_>]) {
        visit_all(self, nodes);
    }

    /// Extract source text from a span
    fn span_text(&self, span: &oxc_span::Span) -> String {
        let start = span.start as usize;
        let end = span.end as usize;
        if start <= end && end <= self.source.len() {
            self.source[start..end].to_string()
        } else {
            String::new()
        }
    }

    fn visit_deferred_triggers(
        &mut self,
        triggers: &oxc_angular_compiler::ast::r3::R3DeferredBlockTriggers<'_>,
    ) {
        // Collect all triggers with their source positions for sorting
        let mut trigger_items: Vec<(u32, Vec<String>)> = Vec::new();

        // Visit when (bound) trigger
        if let Some(trigger) = &triggers.when {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["BoundDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else {
                vec!["BoundDeferredTrigger".to_string(), unparse_expression_r3(&trigger.value)]
            };
            trigger_items.push((trigger.source_span.start, row));
        }
        // Visit idle trigger
        if let Some(trigger) = &triggers.idle {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["IdleDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else if let Some(timeout) = trigger.timeout {
                vec!["IdleDeferredTrigger".to_string(), timeout.to_string()]
            } else {
                vec!["IdleDeferredTrigger".to_string()]
            };
            trigger_items.push((trigger.source_span.start, row));
        }
        // Visit immediate trigger
        if let Some(trigger) = &triggers.immediate {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["ImmediateDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else {
                vec!["ImmediateDeferredTrigger".to_string()]
            };
            trigger_items.push((trigger.source_span.start, row));
        }
        // Visit hover trigger
        if let Some(trigger) = &triggers.hover {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["HoverDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else {
                // Use "null" for None reference, otherwise the reference string
                let reference = trigger
                    .reference
                    .as_ref()
                    .map_or_else(|| "null".to_string(), std::string::ToString::to_string);
                vec!["HoverDeferredTrigger".to_string(), reference]
            };
            trigger_items.push((trigger.source_span.start, row));
        }
        // Visit timer trigger
        if let Some(trigger) = &triggers.timer {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["TimerDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else {
                vec!["TimerDeferredTrigger".to_string(), trigger.delay.to_string()]
            };
            trigger_items.push((trigger.source_span.start, row));
        }
        // Visit interaction trigger
        if let Some(trigger) = &triggers.interaction {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["InteractionDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else {
                // Use "null" for None reference, otherwise the reference string
                let reference = trigger
                    .reference
                    .as_ref()
                    .map_or_else(|| "null".to_string(), std::string::ToString::to_string);
                vec!["InteractionDeferredTrigger".to_string(), reference]
            };
            trigger_items.push((trigger.source_span.start, row));
        }
        // Visit viewport trigger
        if let Some(trigger) = &triggers.viewport {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["ViewportDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else {
                let mut row = vec!["ViewportDeferredTrigger".to_string()];
                if let Some(reference) = &trigger.reference {
                    row.push(reference.to_string());
                } else {
                    row.push("null".to_string());
                }
                if let Some(options) = &trigger.options {
                    row.push(unparse_expression_r3(options));
                }
                row
            };
            trigger_items.push((trigger.source_span.start, row));
        }
        // Visit never trigger
        if let Some(trigger) = &triggers.never {
            let row = if self.mode == HumanizeMode::SourceSpans {
                vec!["NeverDeferredTrigger".to_string(), self.span_text(&trigger.source_span)]
            } else {
                vec!["NeverDeferredTrigger".to_string()]
            };
            trigger_items.push((trigger.source_span.start, row));
        }

        // Sort by source position and output
        trigger_items.sort_by_key(|(pos, _)| *pos);
        for (_, row) in trigger_items {
            self.result.push(row);
        }
    }
}

impl<'a> R3Visitor<'a> for R3Humanizer<'_> {
    fn visit_element(&mut self, element: &R3Element<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            // Source spans mode: [Element, sourceSpan, startSourceSpan, endSourceSpan]
            let source_span = self.span_text(&element.source_span);
            let start_span = self.span_text(&element.start_source_span);
            let end_span = element
                .end_source_span
                .as_ref()
                .map_or_else(|| start_span.clone(), |s| self.span_text(s));
            vec!["Element".to_string(), source_span, start_span, end_span]
        } else {
            // Template transform mode: [Element, name]
            let mut row = vec!["Element".to_string(), element.name.to_string()];
            if element.is_self_closing {
                row.push("#selfClosing".to_string());
            }
            row
        };
        self.result.push(row);

        // Add text attributes
        for attr in &element.attributes {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&attr.source_span);
                let name = attr.name.to_string();
                let value = if attr.value.is_empty() {
                    "<empty>".to_string()
                } else {
                    attr.value.to_string()
                };
                vec!["TextAttribute".to_string(), source_span, name, value]
            } else {
                vec!["TextAttribute".to_string(), attr.name.to_string(), attr.value.to_string()]
            };
            self.result.push(row);
        }

        // Add bound attributes
        for input in &element.inputs {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&input.source_span);
                let name = input.name.to_string();
                // For source spans, output the source text of the value, not the unparsed expression
                let value = input
                    .value_span
                    .as_ref()
                    .map(|s| self.span_text(s))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "<empty>".to_string());
                vec!["BoundAttribute".to_string(), source_span, name, value]
            } else {
                vec![
                    "BoundAttribute".to_string(),
                    binding_type_to_string(input.binding_type),
                    input.name.to_string(),
                    unparse_expression_r3(&input.value),
                ]
            };
            self.result.push(row);
        }

        // Add bound events (only in template transform mode for now)
        if self.mode == HumanizeMode::TemplateTransform {
            for output in &element.outputs {
                // For TwoWay events, the handler should be the original value, not the assignment
                let handler = if output.event_type
                    == oxc_angular_compiler::ast::expression::ParsedEventType::TwoWay
                {
                    // Extract the original value from the assignment (strip " = $event" suffix)
                    let unparsed = unparse_expression_r3(&output.handler);
                    unparsed.strip_suffix(" = $event").unwrap_or(&unparsed).to_string()
                } else {
                    unparse_expression_r3(&output.handler)
                };
                self.result.push(vec![
                    "BoundEvent".to_string(),
                    event_type_to_string(output.event_type),
                    output.name.to_string(),
                    output
                        .target
                        .as_ref()
                        .map_or_else(|| "null".to_string(), std::string::ToString::to_string),
                    handler,
                ]);
            }
        } else {
            // Source spans mode for events
            for output in &element.outputs {
                let source_span = self.span_text(&output.source_span);
                // Determine the name based on event type to match Angular's humanizeSpan(event.keySpan)
                let name = match output.event_type {
                    // For two-way bindings, strip the "Change" suffix to get the original property name
                    oxc_angular_compiler::ast::expression::ParsedEventType::TwoWay => {
                        output.name.strip_suffix("Change").unwrap_or(&output.name).to_string()
                    }
                    // For animation events, include the phase in the name (e.g., "name.done")
                    oxc_angular_compiler::ast::expression::ParsedEventType::LegacyAnimation
                    | oxc_angular_compiler::ast::expression::ParsedEventType::Animation => {
                        if let Some(phase) = &output.phase {
                            format!("{}.{}", output.name, phase)
                        } else {
                            output.name.to_string()
                        }
                    }
                    // Regular events: just use the name
                    oxc_angular_compiler::ast::expression::ParsedEventType::Regular => {
                        output.name.to_string()
                    }
                };
                let handler_text = self.span_text(&output.handler_span);
                self.result.push(vec!["BoundEvent".to_string(), source_span, name, handler_text]);
            }
        }

        // Visit directives
        for directive in &element.directives {
            self.visit_directive(directive);
        }

        // Visit references
        for reference in &element.references {
            self.visit_reference(reference);
        }

        // Visit children
        self.visit_nodes(&element.children);
    }

    fn visit_template(&mut self, template: &R3Template<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&template.source_span);
            let start_span = self.span_text(&template.start_source_span);
            let end_span = template
                .end_source_span
                .as_ref()
                .map_or_else(|| start_span.clone(), |s| self.span_text(s));
            vec!["Template".to_string(), source_span, start_span, end_span]
        } else {
            let mut row = vec!["Template".to_string()];
            if template.is_self_closing {
                row.push("#selfClosing".to_string());
            }
            row
        };
        self.result.push(row);

        // Add text attributes
        for attr in &template.attributes {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&attr.source_span);
                let name = attr.name.to_string();
                let value = if attr.value.is_empty() {
                    "<empty>".to_string()
                } else {
                    attr.value.to_string()
                };
                vec!["TextAttribute".to_string(), source_span, name, value]
            } else {
                vec!["TextAttribute".to_string(), attr.name.to_string(), attr.value.to_string()]
            };
            self.result.push(row);
        }

        // Add bound attributes (hoisted from element)
        for input in &template.inputs {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&input.source_span);
                // For explicit ng-template attributes (key_span has brackets like "[k1]"),
                // use the name field. For inline template microsyntax bindings (key_span
                // is just the keyword like "of"), use the key_span text.
                let key_span_text = self.span_text(&input.key_span);
                let name = if key_span_text.starts_with('[')
                    || key_span_text.starts_with("bind-")
                    || key_span_text.starts_with("data-")
                {
                    // Explicit binding syntax - use the parsed name
                    input.name.to_string()
                } else {
                    // Inline template microsyntax - use key_span text
                    key_span_text
                };
                let value = input
                    .value_span
                    .as_ref()
                    .map(|s| self.span_text(s))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "<empty>".to_string());
                vec!["BoundAttribute".to_string(), source_span, name, value]
            } else {
                vec![
                    "BoundAttribute".to_string(),
                    binding_type_to_string(input.binding_type),
                    input.name.to_string(),
                    unparse_expression_r3(&input.value),
                ]
            };
            self.result.push(row);
        }

        // Add template_attrs (structural directive attributes from microsyntax)
        // These are the directive-specific TextAttributes and BoundAttributes
        for attr in &template.template_attrs {
            match attr {
                R3TemplateAttr::Text(text_attr) => {
                    let row = if self.mode == HumanizeMode::SourceSpans {
                        let source_span = self.span_text(&text_attr.source_span);
                        let name = text_attr.name.to_string();
                        let value = if text_attr.value.is_empty() {
                            "<empty>".to_string()
                        } else {
                            text_attr.value.to_string()
                        };
                        vec!["TextAttribute".to_string(), source_span, name, value]
                    } else {
                        vec![
                            "TextAttribute".to_string(),
                            text_attr.name.to_string(),
                            text_attr.value.to_string(),
                        ]
                    };
                    self.result.push(row);
                }
                R3TemplateAttr::Bound(bound_attr) => {
                    let row = if self.mode == HumanizeMode::SourceSpans {
                        let source_span = self.span_text(&bound_attr.source_span);
                        // Use key_span text for source spans mode
                        let key_span_text = self.span_text(&bound_attr.key_span);
                        let name = if key_span_text.starts_with('[')
                            || key_span_text.starts_with("bind-")
                            || key_span_text.starts_with("data-")
                        {
                            bound_attr.name.to_string()
                        } else {
                            key_span_text
                        };
                        let value = bound_attr
                            .value_span
                            .as_ref()
                            .map(|s| self.span_text(s))
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "<empty>".to_string());
                        vec!["BoundAttribute".to_string(), source_span, name, value]
                    } else {
                        vec![
                            "BoundAttribute".to_string(),
                            binding_type_to_string(bound_attr.binding_type),
                            bound_attr.name.to_string(),
                            unparse_expression_r3(&bound_attr.value),
                        ]
                    };
                    self.result.push(row);
                }
            }
        }

        // Visit variables
        for variable in &template.variables {
            self.visit_variable(variable);
        }

        // Visit references
        for reference in &template.references {
            self.visit_reference(reference);
        }

        // Visit children
        self.visit_nodes(&template.children);
    }

    fn visit_content(&mut self, content: &R3Content<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&content.source_span);
            let start_span = self.span_text(&content.start_source_span);
            let end_span = content
                .end_source_span
                .as_ref()
                .map_or_else(|| start_span.clone(), |s| self.span_text(s));
            vec!["Content".to_string(), source_span, start_span, end_span]
        } else {
            let mut row = vec!["Content".to_string(), content.selector.to_string()];
            if content.is_self_closing {
                row.push("#selfClosing".to_string());
            }
            row
        };
        self.result.push(row);

        // Add text attributes
        for attr in &content.attributes {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&attr.source_span);
                let name = attr.name.to_string();
                let value = if attr.value.is_empty() {
                    "<empty>".to_string()
                } else {
                    attr.value.to_string()
                };
                vec!["TextAttribute".to_string(), source_span, name, value]
            } else {
                vec!["TextAttribute".to_string(), attr.name.to_string(), attr.value.to_string()]
            };
            self.result.push(row);
        }

        // Visit children
        self.visit_nodes(&content.children);
    }

    fn visit_text(&mut self, text: &R3Text<'a>) {
        // Skip whitespace-only text nodes in both modes
        // (Angular filters these out in the humanizer)
        if text.value.trim().is_empty() {
            return;
        }

        let row = if self.mode == HumanizeMode::SourceSpans {
            vec!["Text".to_string(), self.span_text(&text.source_span)]
        } else {
            // Normalize whitespace in template transform mode
            vec!["Text".to_string(), normalize_whitespace(&text.value)]
        };
        self.result.push(row);
    }

    fn visit_bound_text(&mut self, text: &R3BoundText<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            // For source spans, output the source text, not the unparsed expression
            vec!["BoundText".to_string(), self.span_text(&text.source_span)]
        } else {
            vec!["BoundText".to_string(), unparse_expression_r3(&text.value)]
        };
        self.result.push(row);
    }

    fn visit_variable(&mut self, variable: &R3Variable<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&variable.source_span);
            // For implicit context variables (empty source_span), output empty name
            let name =
                if source_span.is_empty() { String::new() } else { variable.name.to_string() };
            let value = variable
                .value_span
                .as_ref()
                .map(|s| self.span_text(s))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "<empty>".to_string());
            vec!["Variable".to_string(), source_span, name, value]
        } else {
            vec!["Variable".to_string(), variable.name.to_string(), variable.value.to_string()]
        };
        self.result.push(row);
    }

    fn visit_reference(&mut self, reference: &R3Reference<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&reference.source_span);
            let name = reference.name.to_string();
            let value = reference
                .value_span
                .as_ref()
                .map(|s| self.span_text(s))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "<empty>".to_string());
            vec!["Reference".to_string(), source_span, name, value]
        } else {
            vec!["Reference".to_string(), reference.name.to_string(), reference.value.to_string()]
        };
        self.result.push(row);
    }

    fn visit_let_declaration(
        &mut self,
        decl: &oxc_angular_compiler::ast::r3::R3LetDeclaration<'a>,
    ) {
        if self.mode == HumanizeMode::SourceSpans {
            // Source spans mode: [LetDeclaration, sourceSpan, name, value]
            // Note: The source span doesn't include the trailing semicolon
            let source_span = self.span_text(&decl.source_span);
            // Strip trailing semicolon from source span if present
            let source_span = source_span.trim_end_matches(';').to_string();
            let name = self.span_text(&decl.name_span);
            let value = self.span_text(&decl.value_span);
            self.result.push(vec!["LetDeclaration".to_string(), source_span, name, value]);
        } else {
            // Template transform mode: [LetDeclaration, name, value]
            let name = decl.name.to_string();
            let value = unparse_expression_r3(&decl.value);
            self.result.push(vec!["LetDeclaration".to_string(), name, value]);
        }
    }

    fn visit_deferred_block(&mut self, block: &oxc_angular_compiler::ast::r3::R3DeferredBlock<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            let end_span =
                block.end_source_span.map_or_else(|| "}".to_string(), |s| self.span_text(&s));
            vec!["DeferredBlock".to_string(), source_span, start_span, end_span]
        } else {
            vec!["DeferredBlock".to_string()]
        };
        self.result.push(row);
        // Visit hydrate triggers first (Angular order: hydrate, regular, prefetch)
        self.visit_deferred_triggers(&block.hydrate_triggers);
        // Visit regular triggers
        self.visit_deferred_triggers(&block.triggers);
        // Visit prefetch triggers
        self.visit_deferred_triggers(&block.prefetch_triggers);
        // Visit children
        for child in &block.children {
            child.visit(self);
        }
        // Visit placeholder, loading, error blocks
        if let Some(placeholder) = &block.placeholder {
            self.visit_deferred_block_placeholder(placeholder);
        }
        if let Some(loading) = &block.loading {
            self.visit_deferred_block_loading(loading);
        }
        if let Some(error) = &block.error {
            self.visit_deferred_block_error(error);
        }
    }

    fn visit_deferred_block_placeholder(
        &mut self,
        block: &oxc_angular_compiler::ast::r3::R3DeferredBlockPlaceholder<'a>,
    ) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            let end_span =
                block.end_source_span.map_or_else(|| "}".to_string(), |s| self.span_text(&s));
            vec!["DeferredBlockPlaceholder".to_string(), source_span, start_span, end_span]
        } else {
            let mut row = vec!["DeferredBlockPlaceholder".to_string()];
            // Add minimum time parameter if present
            if let Some(minimum_time) = block.minimum_time {
                row.push(format!("minimum {minimum_time}ms"));
            }
            row
        };
        self.result.push(row);
        for child in &block.children {
            child.visit(self);
        }
    }

    fn visit_deferred_block_loading(
        &mut self,
        block: &oxc_angular_compiler::ast::r3::R3DeferredBlockLoading<'a>,
    ) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            let end_span =
                block.end_source_span.map_or_else(|| "}".to_string(), |s| self.span_text(&s));
            vec!["DeferredBlockLoading".to_string(), source_span, start_span, end_span]
        } else {
            let mut row = vec!["DeferredBlockLoading".to_string()];
            // Add after time parameter if present
            if let Some(after_time) = block.after_time {
                row.push(format!("after {after_time}ms"));
            }
            // Add minimum time parameter if present
            if let Some(minimum_time) = block.minimum_time {
                row.push(format!("minimum {minimum_time}ms"));
            }
            row
        };
        self.result.push(row);
        for child in &block.children {
            child.visit(self);
        }
    }

    fn visit_deferred_block_error(
        &mut self,
        block: &oxc_angular_compiler::ast::r3::R3DeferredBlockError<'a>,
    ) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            let end_span =
                block.end_source_span.map_or_else(|| "}".to_string(), |s| self.span_text(&s));
            vec!["DeferredBlockError".to_string(), source_span, start_span, end_span]
        } else {
            vec!["DeferredBlockError".to_string()]
        };
        self.result.push(row);
        for child in &block.children {
            child.visit(self);
        }
    }

    fn visit_switch_block(&mut self, block: &oxc_angular_compiler::ast::r3::R3SwitchBlock<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            let end_span =
                block.end_source_span.map_or_else(|| "}".to_string(), |s| self.span_text(&s));
            vec!["SwitchBlock".to_string(), source_span, start_span, end_span]
        } else {
            vec!["SwitchBlock".to_string(), unparse_expression_r3(&block.expression)]
        };
        self.result.push(row);
        for group in &block.groups {
            self.visit_switch_block_case_group(group);
        }
    }

    fn visit_switch_block_case_group(
        &mut self,
        group: &oxc_angular_compiler::ast::r3::R3SwitchBlockCaseGroup<'a>,
    ) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&group.source_span);
            let start_span = self.span_text(&group.start_source_span);
            vec!["SwitchBlockCaseGroup".to_string(), source_span, start_span]
        } else {
            vec!["SwitchBlockCaseGroup".to_string()]
        };
        self.result.push(row);
        for case in &group.cases {
            self.visit_switch_block_case(case);
        }
        for child in &group.children {
            child.visit(self);
        }
    }

    fn visit_switch_block_case(
        &mut self,
        case: &oxc_angular_compiler::ast::r3::R3SwitchBlockCase<'a>,
    ) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&case.source_span);
            let start_span = self.span_text(&case.start_source_span);
            vec!["SwitchBlockCase".to_string(), source_span, start_span]
        } else {
            let expr = case
                .expression
                .as_ref()
                .map_or_else(|| "null".to_string(), |e| unparse_expression_r3(e));
            vec!["SwitchBlockCase".to_string(), expr]
        };
        self.result.push(row);
    }

    fn visit_for_loop_block(&mut self, block: &oxc_angular_compiler::ast::r3::R3ForLoopBlock<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            let end_span =
                block.end_source_span.map_or_else(|| "}".to_string(), |s| self.span_text(&s));
            vec!["ForLoopBlock".to_string(), source_span, start_span, end_span]
        } else {
            // [ForLoopBlock, expression, trackBy]
            vec![
                "ForLoopBlock".to_string(),
                unparse_expression_r3(&block.expression.ast),
                unparse_expression_r3(&block.track_by.ast),
            ]
        };
        self.result.push(row);
        // Visit item variable, context variables, then children
        self.visit_variable(&block.item);
        for var in &block.context_variables {
            self.visit_variable(var);
        }
        for child in &block.children {
            child.visit(self);
        }
        if let Some(empty) = &block.empty {
            self.visit_for_loop_block_empty(empty);
        }
    }

    fn visit_for_loop_block_empty(
        &mut self,
        block: &oxc_angular_compiler::ast::r3::R3ForLoopBlockEmpty<'a>,
    ) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            vec!["ForLoopBlockEmpty".to_string(), source_span, start_span]
        } else {
            vec!["ForLoopBlockEmpty".to_string()]
        };
        self.result.push(row);
        for child in &block.children {
            child.visit(self);
        }
    }

    fn visit_if_block(&mut self, block: &oxc_angular_compiler::ast::r3::R3IfBlock<'a>) {
        // Output [IfBlock] first, then visit branches
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&block.source_span);
            let start_span = self.span_text(&block.start_source_span);
            let end_span =
                block.end_source_span.map_or_else(|| "<empty>".to_string(), |s| self.span_text(&s));
            vec!["IfBlock".to_string(), source_span, start_span, end_span]
        } else {
            vec!["IfBlock".to_string()]
        };
        self.result.push(row);
        for branch in &block.branches {
            self.visit_if_block_branch(branch);
        }
    }

    fn visit_if_block_branch(
        &mut self,
        branch: &oxc_angular_compiler::ast::r3::R3IfBlockBranch<'a>,
    ) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&branch.source_span);
            let start_span = self.span_text(&branch.start_source_span);
            vec!["IfBlockBranch".to_string(), source_span, start_span]
        } else {
            let expr = branch
                .expression
                .as_ref()
                .map_or_else(|| "null".to_string(), |e| unparse_expression_r3(e));
            vec!["IfBlockBranch".to_string(), expr]
        };
        self.result.push(row);

        // Emit variable for expression alias (from "as foo" syntax)
        if let Some(ref alias) = branch.expression_alias {
            self.visit_variable(alias);
        }

        for child in &branch.children {
            child.visit(self);
        }
    }

    fn visit_icu(&mut self, icu: &oxc_angular_compiler::ast::r3::R3Icu<'a>) {
        // Output [Icu, sourceSpan] in source spans mode
        if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&icu.source_span);
            self.result.push(vec!["Icu".to_string(), source_span]);

            // Collect and sort vars by source position (descending - nested vars first)
            let mut vars: Vec<_> = icu.vars.iter().collect();
            vars.sort_by_key(|(_, bound_text)| std::cmp::Reverse(bound_text.source_span.start));
            for (_, bound_text) in vars {
                // Output the actual source text (e.g., "item.var") not the placeholder name (e.g., "VAR_PLURAL")
                let source_text = self.span_text(&bound_text.source_span);
                self.result.push(vec!["Icu:Var".to_string(), source_text]);
            }

            // Collect and sort placeholders by source position
            let mut placeholders: Vec<_> = icu.placeholders.iter().collect();
            placeholders.sort_by_key(|(_, p)| match p {
                oxc_angular_compiler::ast::r3::R3IcuPlaceholder::Text(text) => {
                    text.source_span.start
                }
                oxc_angular_compiler::ast::r3::R3IcuPlaceholder::BoundText(bound) => {
                    bound.source_span.start
                }
            });
            for (_, placeholder) in placeholders {
                match placeholder {
                    oxc_angular_compiler::ast::r3::R3IcuPlaceholder::Text(text) => {
                        let span_text = self.span_text(&text.source_span);
                        self.result.push(vec!["Icu:Placeholder".to_string(), span_text]);
                    }
                    oxc_angular_compiler::ast::r3::R3IcuPlaceholder::BoundText(bound) => {
                        let span_text = self.span_text(&bound.source_span);
                        self.result.push(vec!["Icu:Placeholder".to_string(), span_text]);
                    }
                }
            }
        }
    }

    fn visit_component(&mut self, component: &oxc_angular_compiler::ast::r3::R3Component<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&component.source_span);
            let start_span = self.span_text(&component.start_source_span);
            let end_span = component
                .end_source_span
                .as_ref()
                .map_or_else(|| start_span.clone(), |s| self.span_text(s));
            vec!["Component".to_string(), source_span, start_span, end_span]
        } else {
            let mut row = vec![
                "Component".to_string(),
                component.component_name.to_string(),
                component
                    .tag_name
                    .as_ref()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default(),
                component.full_name.to_string(),
            ];
            if component.is_self_closing {
                row.push("#selfClosing".to_string());
            }
            row
        };
        self.result.push(row);

        // Visit attributes, inputs, outputs, directives, references, children
        for attr in &component.attributes {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&attr.source_span);
                let name = attr.name.to_string();
                let value = if attr.value.is_empty() {
                    "<empty>".to_string()
                } else {
                    attr.value.to_string()
                };
                vec!["TextAttribute".to_string(), source_span, name, value]
            } else {
                vec!["TextAttribute".to_string(), attr.name.to_string(), attr.value.to_string()]
            };
            self.result.push(row);
        }

        for input in &component.inputs {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&input.source_span);
                // Use key_span text for the original binding key (e.g., "of" not "ngForOf")
                let name = self.span_text(&input.key_span);
                let value = input
                    .value_span
                    .as_ref()
                    .map(|s| self.span_text(s))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "<empty>".to_string());
                vec!["BoundAttribute".to_string(), source_span, name, value]
            } else {
                vec![
                    "BoundAttribute".to_string(),
                    binding_type_to_string(input.binding_type),
                    input.name.to_string(),
                    unparse_expression_r3(&input.value),
                ]
            };
            self.result.push(row);
        }

        for output in &component.outputs {
            if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&output.source_span);
                let name = if output.event_type
                    == oxc_angular_compiler::ast::expression::ParsedEventType::TwoWay
                {
                    output.name.strip_suffix("Change").unwrap_or(&output.name).to_string()
                } else {
                    output.name.to_string()
                };
                let handler_text = self.span_text(&output.handler_span);
                self.result.push(vec!["BoundEvent".to_string(), source_span, name, handler_text]);
            } else {
                let handler = if output.event_type
                    == oxc_angular_compiler::ast::expression::ParsedEventType::TwoWay
                {
                    let unparsed = unparse_expression_r3(&output.handler);
                    unparsed.strip_suffix(" = $event").unwrap_or(&unparsed).to_string()
                } else {
                    unparse_expression_r3(&output.handler)
                };
                self.result.push(vec![
                    "BoundEvent".to_string(),
                    event_type_to_string(output.event_type),
                    output.name.to_string(),
                    output
                        .target
                        .as_ref()
                        .map_or_else(|| "null".to_string(), std::string::ToString::to_string),
                    handler,
                ]);
            }
        }

        for directive in &component.directives {
            self.visit_directive(directive);
        }

        for reference in &component.references {
            self.visit_reference(reference);
        }

        self.visit_nodes(&component.children);
    }

    fn visit_directive(&mut self, directive: &oxc_angular_compiler::ast::r3::R3Directive<'a>) {
        let row = if self.mode == HumanizeMode::SourceSpans {
            let source_span = self.span_text(&directive.source_span);
            let start_span = self.span_text(&directive.start_source_span);
            let end_span = directive
                .end_source_span
                .as_ref()
                .map_or_else(|| "<empty>".to_string(), |s| self.span_text(s));
            vec!["Directive".to_string(), source_span, start_span, end_span]
        } else {
            vec!["Directive".to_string(), directive.name.to_string()]
        };
        self.result.push(row);

        // Visit attributes, inputs, outputs, references
        for attr in &directive.attributes {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&attr.source_span);
                let name = attr.name.to_string();
                let value = if attr.value.is_empty() {
                    "<empty>".to_string()
                } else {
                    attr.value.to_string()
                };
                vec!["TextAttribute".to_string(), source_span, name, value]
            } else {
                vec!["TextAttribute".to_string(), attr.name.to_string(), attr.value.to_string()]
            };
            self.result.push(row);
        }

        for input in &directive.inputs {
            let row = if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&input.source_span);
                // Use the name field directly (without brackets), matching element handling
                let name = input.name.to_string();
                let value = input
                    .value_span
                    .as_ref()
                    .map(|s| self.span_text(s))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "<empty>".to_string());
                vec!["BoundAttribute".to_string(), source_span, name, value]
            } else {
                vec![
                    "BoundAttribute".to_string(),
                    binding_type_to_string(input.binding_type),
                    input.name.to_string(),
                    unparse_expression_r3(&input.value),
                ]
            };
            self.result.push(row);
        }

        for output in &directive.outputs {
            if self.mode == HumanizeMode::SourceSpans {
                let source_span = self.span_text(&output.source_span);
                let name = output.name.to_string();
                let handler_text = self.span_text(&output.handler_span);
                self.result.push(vec!["BoundEvent".to_string(), source_span, name, handler_text]);
            } else {
                self.result.push(vec![
                    "BoundEvent".to_string(),
                    event_type_to_string(output.event_type),
                    output.name.to_string(),
                    output
                        .target
                        .as_ref()
                        .map_or_else(|| "null".to_string(), std::string::ToString::to_string),
                    unparse_expression_r3(&output.handler),
                ]);
            }
        }

        for reference in &directive.references {
            self.visit_reference(reference);
        }
    }
}

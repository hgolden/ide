//! Graph Controller.
//!
//! This controller provides access to a specific graph. It lives under a module controller, as
//! each graph belongs to some module.
pub mod executed;

use crate::prelude::*;

use crate::double_representation::connection;
use crate::double_representation::definition;
use crate::double_representation::graph::GraphInfo;
use crate::double_representation::identifier::LocatedName;
use crate::double_representation::identifier::NormalizedName;
use crate::double_representation::identifier::generate_name;
use crate::double_representation::module;
use crate::double_representation::node;
use crate::double_representation::node::NodeInfo;
use crate::model::module::NodeMetadata;

use ast::crumbs::InfixCrumb;
use enso_protocol::language_server;
use parser::Parser;
use span_tree::SpanTree;
use span_tree::action::Actions;
use span_tree::action::Action;
use span_tree::generate::Context as SpanTreeContext;
use span_tree::generate::context::CalledMethodInfo;

pub use crate::double_representation::graph::LocationHint;
pub use crate::double_representation::graph::Id;



// ==============
// === Errors ===
// ==============

/// Error raised when node with given Id was not found in the graph's body.
#[derive(Clone,Copy,Debug,Fail)]
#[fail(display="Node with Id {} was not found.", _0)]
pub struct NodeNotFound(ast::Id);

/// Error raised when an attempt to set node's expression to a binding has been made.
#[derive(Clone,Debug,Fail)]
#[fail(display="Illegal string `{}` given for node expression. It must not be a binding.", _0)]
pub struct BindingExpressionNotAllowed(String);

/// Expression AST cannot be used to produce a node. Means a bug in parser and id-giving code.
#[derive(Clone,Copy,Debug,Fail)]
#[fail(display="Internal error: failed to create a new node.")]
pub struct FailedToCreateNode;

#[allow(missing_docs)]
#[derive(Clone,Copy,Debug,Fail)]
#[fail(display="Source node {} has no pattern, so it cannot form connections.",node)]
pub struct NoPatternOnNode {
    pub node : node::Id,
}



// ====================
// === Notification ===
// ====================

/// A notification about changes of a specific graph in a module.
#[derive(Copy,Clone,Debug,Eq,PartialEq)]
pub enum Notification {
    /// The content should be fully reloaded.
    Invalidate,
    /// The graph node's ports need updating, e.g., types, names.
    PortsUpdate,
}



// ============
// === Node ===
// ============

/// Description of the node with all information available to the graph controller.
#[derive(Clone,Debug)]
pub struct Node {
    /// Information based on AST, from double_representation module.
    pub info : NodeInfo,
    /// Information about this node stored in the module's metadata.
    pub metadata : Option<NodeMetadata>,
}

impl Node {
	/// Check if node has a specific position set in metadata.
    pub fn has_position(&self) -> bool {
        self.metadata.as_ref().map_or(false, |m| m.position.is_some())
    }
}



// ===================
// === NewNodeInfo ===
// ===================

/// Describes the node to be added.
#[derive(Clone,Debug)]
pub struct NewNodeInfo {
    /// Expression to be placed on the node
    pub expression : String,
    /// Visual node position in the graph scene.
    pub metadata : Option<NodeMetadata>,
    /// ID to be given to the node.
    pub id : Option<ast::Id>,
    /// Where line created by adding this node should appear.
    pub location_hint : LocationHint,
    /// Introduce variable name for the node, making it into an assignment line.
    pub introduce_pattern : bool,

}

impl NewNodeInfo {
    /// New node with given expression added at the end of the graph's blocks.
    pub fn new_pushed_back(expression:impl Str) -> NewNodeInfo {
        NewNodeInfo {
            expression        : expression.into(),
            metadata          : default(),
            id                : default(),
            location_hint     : LocationHint::End,
            introduce_pattern : default(),
        }
    }
}



// ===================
// === Connections ===
// ===================

/// Reference to the port (i.e. the span tree node).
pub type PortRef<'a> = span_tree::node::Ref<'a>;


// === Endpoint

/// Connection endpoint - a port on a node, described using span-tree crumbs.
#[allow(missing_docs)]
#[derive(Clone,Debug,Default,Eq,Hash,PartialEq)]
pub struct Endpoint {
    pub node : double_representation::node::Id,
    pub port : span_tree::Crumbs,
    /// Crumbs which locate the Var in the `port` ast node.
    ///
    /// In normal case this is an empty crumb (which means that the whole span of `port` is the
    /// mentioned Var. However, span tree does not cover all the possible ast of node expression
    /// (e.g. it does not decompose Blocks), but still we want to pass information about connection
    /// to such port and be able to remove it.
    pub var_crumbs: ast::Crumbs,
}

impl Endpoint {
    /// Create endpoint with empty `var_crumbs`.
    pub fn new(node:double_representation::node::Id, port:impl Into<span_tree::Crumbs>) -> Self {
        let var_crumbs = default();
        let port       = port.into();
        Endpoint{node,port,var_crumbs}
    }
}


// === Connection ===

/// Connection described using span-tree crumbs.
#[allow(missing_docs)]
#[derive(Clone,Debug,Default,Eq,Hash,PartialEq)]
pub struct Connection {
    pub source      : Endpoint,
    pub destination : Endpoint
}


// === NodeTrees ===

/// Stores node's span trees: one for inputs (expression) and optionally another one for inputs
/// (pattern).
#[derive(Clone,Debug,Default)]
pub struct NodeTrees {
    /// Describes node inputs, i.e. its expression.
    pub inputs : SpanTree,
    /// Describes node outputs, i.e. its pattern. `None` if a node is not an assignment.
    pub outputs : Option<SpanTree>,
}

impl NodeTrees {
    #[allow(missing_docs)]
    pub fn new(node:&NodeInfo, context:&impl SpanTreeContext) -> Option<NodeTrees> {
        let inputs  = SpanTree::new(node.expression(),context).ok()?;
        let outputs = if let Some(pat) = node.pattern() {
            Some(SpanTree::new(pat,context).ok()?)
        } else {
            None
        };
        Some(NodeTrees {inputs,outputs})
    }

    /// Converts AST crumbs (as obtained from double rep's connection endpoint) into the
    /// appriopriate span-tree node reference.
    pub fn get_span_tree_node<'a,'b>(&'a self, ast_crumbs:&'b [ast::Crumb])
    -> Option<span_tree::node::NodeFoundByAstCrumbs<'a,'b>> {
        if let Some(outputs) = self.outputs.as_ref() {
            // Node in assignment form. First crumb decides which span tree to use.
            let tree = match ast_crumbs.get(0) {
                Some(ast::crumbs::Crumb::Infix(InfixCrumb::LeftOperand))  => Some(outputs),
                Some(ast::crumbs::Crumb::Infix(InfixCrumb::RightOperand)) => Some(&self.inputs),
                _ => None,
            };
            tree.and_then(|tree| tree.root_ref().get_descendant_by_ast_crumbs(&ast_crumbs[1..]))
        } else {
            // Expression node - there is only inputs span tree.
            self.inputs.root_ref().get_descendant_by_ast_crumbs(ast_crumbs)
        }
    }
}


// === Connections ===

/// Describes connections in the graph. For convenience also includes information about port
/// structure of the involved nodes.
#[derive(Clone,Debug,Default)]
pub struct Connections {
    /// Span trees for all nodes that have connections.
    pub trees       : HashMap<node::Id,NodeTrees>,
    /// The connections between nodes in the graph.
    pub connections : Vec<Connection>,
}

impl Connections {
    /// Describes a connection for given double representation graph.
    pub fn new(graph:&GraphInfo, context:&impl SpanTreeContext) -> Connections {
        let trees = graph.nodes().iter().filter_map(|node| {
            Some((node.id(), NodeTrees::new(node,context)?))
        }).collect();

        let mut ret     = Connections {trees, connections:default()};
        let connections = graph.connections().into_iter().filter_map(|c|
            ret.convert_connection(&c)
        ).collect();
        ret.connections = connections;
        ret
    }

    /// Converts Endpoint from double representation to the span tree crumbs.
    pub fn convert_endpoint
    (&self, endpoint:&double_representation::connection::Endpoint) -> Option<Endpoint> {
        let tree           = self.trees.get(&endpoint.node)?;
        let span_tree_node = tree.get_span_tree_node(&endpoint.crumbs)?;
        Some(Endpoint{
            node       : endpoint.node,
            port       : span_tree_node.node.crumbs,
            var_crumbs : span_tree_node.ast_crumbs.into(),
        })
    }

    /// Converts Connection from double representation to the span tree crumbs.
    pub fn convert_connection
    (&self, connection:&double_representation::connection::Connection) -> Option<Connection> {
        let source = self.convert_endpoint(&connection.source)?;
        let destination = self.convert_endpoint(&connection.destination)?;
        Some(Connection {
            source      ,
            destination ,
        })
    }
}



// =================
// === Utilities ===
// =================

/// Suggests a variable name for storing results of the given expression.
///
/// Name will try to express result of an infix operation (`sum` for `a+b`), kind of literal
/// (`number` for `5`) and target function name for prefix chain.
///
/// The generated name is not unique and might collide with already present identifiers.
pub fn name_for_ast(ast:&Ast) -> String {
    use ast::*;
    match ast.shape() {
        Shape::Var          (ident) => ident.name.clone(),
        Shape::Cons         (ident) => ident.name.to_lowercase(),
        Shape::Number       (_)     => "number".into(),
        Shape::DanglingBase (_)     => "number".into(),
        Shape::TextLineRaw  (_)     => "text".into(),
        Shape::TextLineFmt  (_)     => "text".into(),
        Shape::TextBlockRaw (_)     => "text".into(),
        Shape::TextBlockFmt (_)     => "text".into(),
        Shape::TextUnclosed (_)     => "text".into(),
        Shape::Opr          (opr)   => {
            match opr.name.as_ref() {
                "+" => "sum",
                "*" => "product",
                "-" => "difference",
                "/" => "quotient",
                _   => "operator",
            }.into()
        }
        _ => {
            if let Some(infix) = ast::opr::GeneralizedInfix::try_new(ast) {
                name_for_ast(infix.opr.ast())
            } else if let Some(prefix) = ast::prefix::Chain::from_ast(ast) {
                name_for_ast(&prefix.func)
            } else {
                "var".into()
            }
        }
    }
}



// ====================
// === EndpointInfo ===
// ====================

/// Helper structure for controller that describes known information about a connection's endpoint.
///
/// Also provides a number of utility functions for connection operations.
#[derive(Clone,Debug)]
pub struct EndpointInfo {
    /// The endpoint descriptor.
    pub endpoint  : Endpoint,
    /// Ast of the relevant node piece (expression or the pattern).
    pub ast       : Ast,
    /// Span tree for the relevant node side (outputs or inputs).
    pub span_tree : SpanTree,
}

impl EndpointInfo {
    /// Construct information about endpoint. Ast must be the node's expression or pattern.
    pub fn new
    (endpoint:&Endpoint, ast:&Ast, context:&impl SpanTreeContext)
    -> FallibleResult<EndpointInfo> {
        Ok(EndpointInfo {
            endpoint  : endpoint.clone(),
            ast       : ast.clone(),
            span_tree : SpanTree::new(ast,context)?,
        })
    }

    /// Obtains a reference to the port (span tree node) of this endpoint.
    pub fn port(&self) -> FallibleResult<span_tree::node::Ref> {
        self.span_tree.get_node(&self.endpoint.port)
    }

    /// Obtain reference to the parent of the port identified by given crumbs slice.
    pub fn parent_port_of(&self, crumbs:&[span_tree::node::Crumb]) -> Option<PortRef> {
        let parent_crumbs = span_tree::node::parent_crumbs(crumbs);
        parent_crumbs.and_then(|cr| self.span_tree.get_node(cr.iter()).ok())
    }

    /// Iterates over sibling ports located after this endpoint in its chain.
    pub fn chained_ports_after<'a>(&'a self) -> impl Iterator<Item = PortRef> + 'a {
        let parent_port = self.parent_chain_port();
        let ports_after = parent_port.map(move |parent_port|
            parent_port.chain_children_iter().skip_while(move |port|
                port.crumbs != self.endpoint.port
            ).skip(1));

        ports_after.into_iter().flatten()
    }

    /// Obtains parent port. If this port is part of chain, the parent port will be the parent of
    /// the whole chain.
    pub fn parent_chain_port(&self) -> Option<PortRef> {
        // TODO [mwu]
        //  Unpleasant. Likely there should be something in span tree that allows obtaining
        //  sequence of nodes between root and given crumb. Or sth.
        let mut parent_port = self.parent_port_of(&self.endpoint.port);
        while parent_port.contains_if(|p| p.node.kind == span_tree::node::Kind::Chained) {
            parent_port = parent_port.and_then(|p| self.parent_port_of(&p.crumbs));
        }
        parent_port
    }

    /// Ast being the exact endpoint target. Might be more granular than a span tree port.
    pub fn target_ast(&self) -> FallibleResult<&Ast> {
        self.ast.get_traversing(&self.full_ast_crumbs()?)
    }

    /// Full sequence of Ast crumbs identifying endpoint target.
    pub fn full_ast_crumbs(&self) -> FallibleResult<ast::Crumbs> {
        let port       = self.port()?;
        let mut crumbs = port.ast_crumbs;
        crumbs.extend(self.endpoint.var_crumbs.iter().cloned());
        Ok(crumbs)
    }

    /// Sets AST at the given port. Returns new root Ast.
    pub fn set(&self, ast_to_set:Ast) -> FallibleResult<Ast> {
        self.port()?.set(&self.ast,ast_to_set)
    }

    /// Sets AST at the endpoint target. Returns new root Ast. Does not use span tree logic.
    pub fn set_ast(&self, ast_to_set:Ast) -> FallibleResult<Ast> {
        self.ast.set_traversing(&self.full_ast_crumbs()?,ast_to_set)
    }

    /// Erases given port. Returns new root Ast.
    pub fn erase(&self) -> FallibleResult<Ast> {
        self.port()?.erase(&self.ast)
    }
}



// ==================
// === Controller ===
// ==================

/// Handle providing graph controller interface.
#[derive(Clone,CloneRef,Debug)]
#[allow(missing_docs)]
pub struct Handle {
    /// Identifier of the graph accessed through this controller.
    pub id            : Rc<Id>,
    pub module        : model::Module,
    pub suggestion_db : Rc<model::SuggestionDatabase>,
    parser            : Parser,
    logger            : Logger,
}

impl Handle {

    /// Creates a new controller. Does not check if id is valid.
    pub fn new_unchecked
    ( parent        : impl AnyLogger
    , module        : model::Module
    , suggestion_db : Rc<model::SuggestionDatabase>
    , parser        : Parser
    , id            : Id
    ) -> Handle {
        let id     = Rc::new(id);
        let logger = Logger::sub(parent,format!("Graph Controller {}", id));
        Handle {id,module,suggestion_db,parser,logger}
    }

    /// Create a new graph controller. Given ID should uniquely identify a definition in the
    /// module. Fails if ID cannot be resolved.
    pub fn new
    ( parent        : impl AnyLogger
    , module        : model::Module
    , suggestion_db : Rc<model::SuggestionDatabase>
    , parser        : Parser
    , id            : Id
    ) -> FallibleResult<Handle> {
        let ret = Self::new_unchecked(parent,module,suggestion_db,parser,id);
        // Get and discard definition info, we are just making sure it can be obtained.
        let _ = ret.graph_definition_info()?;
        Ok(ret)
    }

    /// Create a graph controller for the given method.
    ///
    /// Fails if the module is inaccessible or if the module does not contain the given method.
    pub async fn new_method
    (parent:impl AnyLogger, project:&model::Project, method:&language_server::MethodPointer)
    -> FallibleResult<controller::Graph> {
        let method      = method.clone();
        let root_id     = project.content_root_id();
        let module_path = model::module::Path::from_method(root_id,&method)?;
        let module      = project.module(module_path).await?;
        let definition  = module.lookup_method(project.name().as_ref(),&method)?;
        Self::new(parent,module,project.suggestion_db(),project.parser(),definition)
    }

    /// Retrieves double rep information about definition providing this graph.
    pub fn graph_definition_info
    (&self) -> FallibleResult<double_representation::definition::DefinitionInfo> {
        self.module.find_definition(&self.id)
    }

    /// Get the double representation description of the graph.
    pub fn graph_info(&self) -> FallibleResult<GraphInfo> {
        self.graph_definition_info().map(GraphInfo::from_definition)
    }

    /// Returns double rep information about all nodes in the graph.
    pub fn all_node_infos(&self) -> FallibleResult<Vec<NodeInfo>> {
        let graph = self.graph_info()?;
        Ok(graph.nodes())
    }

    /// Retrieves double rep information about node with given ID.
    pub fn node_info(&self, id:ast::Id) -> FallibleResult<NodeInfo> {
        let nodes = self.all_node_infos()?;
        let node  = nodes.into_iter().find(|node_info| node_info.id() == id);
        node.ok_or_else(|| NodeNotFound(id).into())
    }

    /// Gets information about node with given id.
    ///
    /// Note that it is more efficient to use `get_nodes` to obtain all information at once,
    /// rather then repeatedly call this method.
    pub fn node(&self, id:ast::Id) -> FallibleResult<Node> {
        let info     = self.node_info(id)?;
        let metadata = self.module.node_metadata(id).ok();
        Ok(Node {info,metadata})
    }

    /// Returns information about all the nodes currently present in this graph.
    pub fn nodes(&self) -> FallibleResult<Vec<Node>> {
        let node_infos = self.all_node_infos()?;
        let mut nodes  = Vec::new();
        for info in node_infos {
            let metadata = self.module.node_metadata(info.id()).ok();
            nodes.push(Node {info,metadata})
        }
        Ok(nodes)
    }

    /// Returns information about all the connections between graph's nodes.
    ///
    /// The context is used to create all span trees and possible affects the tree structure (so
    /// port ids depend on context).
    ///
    /// To obtain connection using only the locally available data, one may invoke this method
    /// passing `self` (i.e. the call target) as the context.
    pub fn connections
    (&self, context:&impl SpanTreeContext) -> FallibleResult<Connections> {
        let graph = self.graph_info()?;
        Ok(Connections::new(&graph,context))
    }

    /// Suggests a name for a variable that shall store the node value.
    ///
    /// Analyzes the expression, e.g. result for "a+b" shall be named "sum".
    /// The caller should make sure that obtained name won't collide with any symbol usage before
    /// actually introducing it. See `variable_name_for`.
    pub fn variable_name_base_for(node:&NodeInfo) -> String {
        name_for_ast(node.expression())
    }

    /// Identifiers introduced or referred to in the current graph's scope.
    ///
    /// Introducing identifier not included on this list should have no side-effects on the name
    /// resolution in the code in this graph.
    pub fn used_names(&self) -> FallibleResult<Vec<LocatedName>> {
        use double_representation::alias_analysis;
        let def   = self.graph_definition_info()?;
        let body  = def.body();
        let usage = if matches!(body.shape(),ast::Shape::Block(_)) {
            alias_analysis::analyze_crumbable(body.item)
        } else if  let Some(node) = NodeInfo::from_line_ast(&body) {
            alias_analysis::analyze_node(&node)
        } else {
            // Generally speaking - impossible. But if there is no node in the definition
            // body, then there is nothing that could use any symbols, so nothing is used.
            default()
        };
        Ok(usage.all_identifiers())
    }

    /// Suggests a variable name for storing results of the given node. Name will get a number
    /// appended to avoid conflicts with other identifiers used in the graph.
    pub fn variable_name_for(&self, node:&NodeInfo) -> FallibleResult<ast::known::Var> {
        let base_name  = Self::variable_name_base_for(node);
        let used_names = self.used_names()?.into_iter().map(|located_name| located_name.item);
        let name       = generate_name(base_name.as_str(),used_names)?.as_var()?;
        Ok(ast::known::Var::new(name,None))
    }

    /// Converts node to an assignment, where the whole value is bound to a single identifier.
    /// Modifies the node, discarding any previously set pattern.
    /// Returns the identifier with the node's expression value.
    pub fn introduce_name_on(&self, id:node::Id) -> FallibleResult<ast::known::Var> {
        let node = self.node(id)?;
        let name = self.variable_name_for(&node.info)?;
        self.set_pattern_on(id,name.ast().clone())?;
        Ok(name)
    }

    /// Set a new pattern on the node with given id. Discards any previously set pattern.
    pub fn set_pattern_on(&self, id:node::Id, pattern:Ast) -> FallibleResult {
        self.update_node(id, |mut node| {
            node.set_pattern(pattern);
            node
        })
    }

    /// Obtains information for connection's destination endpoint.
    pub fn destination_info
    (&self, connection:&Connection, context:&impl SpanTreeContext) -> FallibleResult<EndpointInfo> {
        let destination_node = self.node_info(connection.destination.node)?;
        let target_node_ast  = destination_node.expression();
        EndpointInfo::new(&connection.destination,target_node_ast,context)
    }

    /// Obtains information about connection's source endpoint.
    pub fn source_info
    (&self, connection:&Connection, context:&impl SpanTreeContext) -> FallibleResult<EndpointInfo> {
        let source_node = self.node_info(connection.source.node)?;
        if let Some(pat) = source_node.pattern() {
            EndpointInfo::new(&connection.source,pat,context)
        } else {
            // For subports we would not have any idea what pattern to introduce. So we fail.
            Err(NoPatternOnNode {node : connection.source.node}.into())
        }
    }

    /// If the node has no pattern, introduces a new pattern with a single variable name.
    pub fn introduce_pattern_if_missing(&self, node:node::Id) -> FallibleResult<Ast> {
        let source_node = self.node_info(node)?;
        if let Some(pat) = source_node.pattern() {
            Ok(pat.clone())
        } else {
            self.introduce_name_on(node).map(|var| var.into())
        }
    }

    /// Reorders lines so the former node is placed after the latter. Does nothing, if the latter
    /// node is already placed after former.
    ///
    /// Additionally all dependent node the `node_to_be_after` being before its new line are also
    /// moved after it, keeping their order.
    pub fn place_node_and_dependencies_lines_after
    (&self, node_to_be_before:node::Id, node_to_be_after:node::Id) -> FallibleResult {
        let definition      = self.graph_definition_info()?;
        let definition_ast  = &definition.body().item;
        let dependent_nodes = connection::dependent_nodes_in_def(definition_ast,node_to_be_after);
        let mut lines       = definition.block_lines()?;

        let before_node_position = node::index_in_lines(&lines,node_to_be_before)?;
        let after_node_position  = node::index_in_lines(&lines,node_to_be_after)?;
        if before_node_position > after_node_position {
            let should_be_at_end = |line:&ast::BlockLine<Option<Ast>>| {
                let id = NodeInfo::from_block_line(line).map(|node| node.id());
                id.map_or(false, |id| id == node_to_be_after || dependent_nodes.contains(&id))
            };
            lines[after_node_position..=before_node_position].sort_by_key(should_be_at_end);
            self.update_definition_ast(|mut def| {
                def.set_block_lines(lines)?;
                Ok(def)
            })?;
        }
        Ok(())
    }

    /// Create connection in graph.
    pub fn connect
    (&self, connection:&Connection, context:&impl SpanTreeContext) -> FallibleResult {
        if connection.source.port.is_empty() {
            // If we create connection from node's expression root, we are able to introduce missing
            // pattern with a new variable.
            self.introduce_pattern_if_missing(connection.source.node)?;
        }

        let source_info              = self.source_info(connection,context)?;
        let destination_info         = self.destination_info(connection,context)?;
        let source_identifier        = source_info.target_ast()?.clone();
        let updated_target_node_expr = destination_info.set(source_identifier)?;
        self.set_expression_ast(connection.destination.node,updated_target_node_expr)?;

        // Reorder node lines, so the connection target is after connection source.
        let source_node      = connection.source.node;
        let destination_node = connection.destination.node;
        self.place_node_and_dependencies_lines_after(source_node,destination_node)
    }

    /// Remove the connections from the graph.
    pub fn disconnect
    (&self, connection:&Connection, context:&impl SpanTreeContext) -> FallibleResult {
        let info = self.destination_info(connection,context)?;

        let updated_expression = if connection.destination.var_crumbs.is_empty() {
            let port                        = info.port()?;
            let only_insertion_points_after = info.chained_ports_after()
                .all(|p| p.node.is_insertion_point());
            if port.is_action_available(Action::Erase) && only_insertion_points_after {
                info.erase()
            } else {
                info.set(Ast::blank())
            }
        } else {
            info.set_ast(Ast::blank())
        }?;

        self.set_expression_ast(connection.destination.node, updated_expression)
    }

    /// Obtain the definition information for this graph from the module's AST.
    pub fn definition(&self) -> FallibleResult<definition::ChildDefinition> {
        let module_ast = self.module.ast();
        module::locate(&module_ast, &self.id)
    }

    /// Updates the AST of the definition of this graph.
    pub fn update_definition_ast<F>(&self, f:F) -> FallibleResult
    where F:FnOnce(definition::DefinitionInfo) -> FallibleResult<definition::DefinitionInfo> {
        let ast_so_far     = self.module.ast();
        let definition     = self.definition()?;
        let new_definition = f(definition.item)?;
        info!(self.logger, "Applying graph changes onto definition");
        let new_ast    = new_definition.ast.into();
        let new_module = ast_so_far.set_traversing(&definition.crumbs,new_ast)?;
        self.module.update_ast(new_module)
    }

    /// Parses given text as a node expression.
    pub fn parse_node_expression
    (&self, expression_text:impl Str) -> FallibleResult<Ast> {
        let node_ast      = self.parser.parse_line(expression_text.as_ref())?;
        if ast::opr::is_assignment(&node_ast) {
            Err(BindingExpressionNotAllowed(expression_text.into()).into())
        } else {
            Ok(node_ast)
        }
    }

    /// Adds a new node to the graph and returns information about created node.
    pub fn add_node(&self, node:NewNodeInfo) -> FallibleResult<ast::Id> {
        info!(self.logger, "Adding node with expression `{node.expression}`");
        let ast           = self.parse_node_expression(&node.expression)?;
        let mut node_info = node::NodeInfo::from_line_ast(&ast).ok_or(FailedToCreateNode)?;
        if let Some(desired_id) = node.id {
            node_info.set_id(desired_id)
        }

        if node.introduce_pattern && node_info.pattern().is_none() {
            let var = self.variable_name_for(&node_info)?;
            node_info.set_pattern(var.into());
        }

        self.update_definition_ast(|definition| {
            let mut graph = GraphInfo::from_definition(definition);
            let node_ast  = node_info.ast().clone();
            graph.add_node(node_ast,node.location_hint)?;
            Ok(graph.source)
        })?;

        if let Some(initial_metadata) = node.metadata {
            self.module.set_node_metadata(node_info.id(),initial_metadata)?;
        }

        Ok(node_info.id())
    }

    /// Removes the node with given Id.
    pub fn remove_node(&self, id:ast::Id) -> FallibleResult {
        info!(self.logger, "Removing node {id}");
        self.update_definition_ast(|definition| {
            let mut graph = GraphInfo::from_definition(definition);
            graph.remove_node(id)?;
            Ok(graph.source)
        })?;

        // It's fine if there were no metadata.
        let _ = self.module.remove_node_metadata(id);
        Ok(())
    }

    /// Sets the given's node expression.
    pub fn set_expression(&self, id:ast::Id, expression_text:impl Str) -> FallibleResult {
        info!(self.logger, "Setting node {id} expression to `{expression_text.as_ref()}`");
        let new_expression_ast = self.parse_node_expression(expression_text)?;
        self.set_expression_ast(id,new_expression_ast)
    }

    /// Sets the given's node expression.
    pub fn set_expression_ast(&self, id:ast::Id, expression:Ast) -> FallibleResult {
        info!(self.logger, "Setting node {id} expression to `{expression.repr()}`");
        self.update_definition_ast(|definition| {
            let mut graph = GraphInfo::from_definition(definition);
            graph.edit_node(id,expression)?;
            Ok(graph.source)
        })?;
        Ok(())
    }

    /// Collapses the selected nodes.
    ///
    /// Lines corresponding to the selection will be extracted to a new method definition.
    pub fn collapse
    (&self, nodes:impl IntoIterator<Item=node::Id>, new_method_name_base:&str)
    -> FallibleResult<node::Id> {
        analytics::remote_log_event("graph::collapse");
        use double_representation::refactorings::collapse::collapse;
        use double_representation::refactorings::collapse::Collapsed;
        let nodes : Vec<_> = Result::from_iter(nodes.into_iter().map(|id| self.node(id)))?;
        info!(self.logger, "Collapsing {nodes:?}.");
        let collapsed_positions = nodes.iter().filter_map(|node| {
            node.metadata.as_ref().and_then(|metadata| metadata.position)
        });
        let ast             = self.module.ast();
        let mut module      = module::Info {ast};
        let introduced_name = module.generate_name(new_method_name_base)?;
        let node_ids        = nodes.iter().map(|node| node.info.id());
        let graph           = self.graph_info()?;
        let collapsed       = collapse(&graph,node_ids,introduced_name,&self.parser)?;
        let Collapsed {new_method,updated_definition,collapsed_node} = collapsed;

        let graph   = self.graph_info()?;
        let my_name = graph.source.name.item;
        module.add_method(new_method,module::Placement::Before(my_name),&self.parser)?;
        self.module.update_ast(module.ast)?;
        self.update_definition_ast(|_| Ok(updated_definition))?;
        let position = Some(model::module::Position::mean(collapsed_positions));
        let metadata = NodeMetadata {position,..default()};
        self.module.set_node_metadata(collapsed_node,metadata)?;
        Ok(collapsed_node)
    }

    /// Updates the given node in the definition.
    ///
    /// The function `F` is called with the information with the state of the node so far and
    pub fn update_node<F>(&self, id:ast::Id, f:F) -> FallibleResult
    where F : FnOnce(NodeInfo) -> NodeInfo {
        self.update_definition_ast(|definition| {
            let mut graph = GraphInfo::from_definition(definition);
            graph.update_node(id,|node| {
                let new_node = f(node);
                info!(self.logger, "Setting node {id} line to `{new_node.repr()}`");
                Some(new_node)
            })?;
            Ok(graph.source)
        })?;
        Ok(())
    }

    /// Subscribe to updates about changes in this graph.
    pub fn subscribe(&self) -> impl Stream<Item=Notification> {
        let module_sub = self.module.subscribe().map(|notification| {
            match notification.kind {
                model::module::NotificationKind::Invalidate      |
                model::module::NotificationKind::CodeChanged{..} |
                model::module::NotificationKind::MetadataChanged => Notification::Invalidate,
            }
        });
        let db_sub = self.suggestion_db.subscribe().map(|notification| {
            match notification {
                model::suggestion_database::Notification::Updated => Notification::PortsUpdate
            }
        });
        futures::stream::select(module_sub,db_sub)
    }
}


// === Span Tree Context ===

/// Span Tree generation context for a graph that does not know about execution.
///
/// It just applies the information from the metadata.
impl span_tree::generate::Context for Handle {
    fn call_info(&self, id:node::Id, name:Option<&str>) -> Option<CalledMethodInfo> {
        let db       = &self.suggestion_db;
        let metadata = self.module.node_metadata(id).ok()?;
        let db_entry = db.lookup_method(metadata.intended_method?)?;
        // If the name is different than intended method than apparently it is not intended anymore
        // and should be ignored.
        let matching = if let Some(name) = name {
            NormalizedName::new(name) == NormalizedName::new(&db_entry.name)
        } else {
            true
        };
        matching.then_with(|| db_entry.invocation_info())
    }
}



// ============
// === Test ===
// ============

#[cfg(test)]
pub mod tests {
    use super::*;

    use crate::double_representation::identifier::NormalizedName;
    use crate::executor::test_utils::TestWithLocalPoolExecutor;
    use crate::model::module::Position;

    use ast::crumbs;
    use ast::test_utils::expect_shape;
    use data::text::Index;
    use data::text::TextChange;
    use enso_protocol::language_server::MethodPointer;
    use parser::Parser;
    use utils::test::ExpectTuple;
    use wasm_bindgen_test::wasm_bindgen_test;

    use crate::model::suggestion_database;

    /// Returns information about all the connections between graph's nodes.
    ///
    /// Will use `self` as the context for span tree generation.
    pub fn connections(graph:&Handle) -> FallibleResult<Connections> {
        graph.connections(graph)
    }

    /// All the data needed to set up and run the graph controller in mock environment.
    #[derive(Clone,Debug)]
    pub struct MockData {
        pub module_path  : model::module::Path,
        pub graph_id     : Id,
        pub project_name : String,
        pub code         : String,
        pub suggestions  : HashMap<suggestion_database::entry::Id,suggestion_database::Entry>,
    }

    impl MockData {
        /// Creates a mock data with the `main` function being an inline definition with a single
        /// node.
        pub fn new() -> Self {
            MockData {
                module_path  : crate::test::mock::data::module_path(),
                graph_id     : crate::test::mock::data::graph_id(),
                project_name : crate::test::mock::data::PROJECT_NAME.to_owned(),
                code         : crate::test::mock::data::CODE.to_owned(),
                suggestions  : default(),
            }
        }

        /// Creates a mock data with the main function being an inline definition.
        ///
        /// The single node's expression is taken as the argument.
        pub fn new_inline(main_body:impl AsRef<str>) -> Self {
            let definition_name = crate::test::mock::data::DEFINITION_NAME;
            MockData {
                code : format!("{} = {}",definition_name,main_body.as_ref()),
                ..Self::new()
            }
        }

        pub fn module_data(&self) -> model::module::test::MockData {
            model::module::test::MockData {
                code : self.code.clone(),
                path : self.module_path.clone(),
                ..default()
            }
        }

        /// Create a graph controller from the current mock data.
        pub fn graph(&self) -> Handle {
            let logger      = Logger::new("Test");
            let parser      = Parser::new().unwrap();
            let module      = self.module_data().plain(&parser);
            let id          = self.graph_id.clone();
            let db          = self.suggestion_db();
            Handle::new(logger,module,db,parser,id).unwrap()
        }

        pub fn method(&self) -> MethodPointer {
            self.module_path.method_pointer(&self.project_name,self.graph_id.to_string())
        }

        pub fn suggestion_db(&self) -> Rc<model::SuggestionDatabase> {
            use model::suggestion_database::SuggestionDatabase;
            let entries = self.suggestions.iter();
            Rc::new(SuggestionDatabase::new_from_entries(Logger::new("Test"),entries))
        }
    }

    impl Default for MockData {
        fn default() -> Self {
            Self::new()
        }
    }

    #[derive(Debug,Shrinkwrap)]
    #[shrinkwrap(mutable)]
    pub struct Fixture{
        pub data  : MockData,
        #[shrinkwrap(main_field)]
        pub inner : TestWithLocalPoolExecutor,
    }

    impl Fixture {
        pub fn set_up() -> Fixture {
            let data  = MockData::new();
            let inner = TestWithLocalPoolExecutor::set_up();
            Self {data,inner}
        }

        pub fn run<Test,Fut>(&mut self, test:Test)
        where Test : FnOnce(Handle) -> Fut + 'static,
              Fut  : Future<Output=()> {
            let graph = self.data.graph();
            self.run_task(async move {
                test(graph).await
            })
        }
    }

    #[wasm_bindgen_test]
    fn node_operations() {
        Fixture::set_up().run(|graph| async move {
            let uid     = graph.all_node_infos().unwrap()[0].id();
            let pos     = Position {vector:Vector2::new(0.0,0.0)};
            let updater = Box::new(|data:&mut NodeMetadata| data.position = Some(pos));
            graph.module.with_node_metadata(uid,updater).unwrap();
            assert_eq!(graph.module.node_metadata(uid).unwrap().position, Some(pos));
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_notification_relay() {
        Fixture::set_up().run(|graph| async move {
            let mut sub = graph.subscribe();
            let change  = TextChange::insert(Index::new(12), "2".into());
            graph.module.apply_code_change(change, &graph.parser,default()).unwrap();
            assert_eq!(Some(Notification::Invalidate), sub.next().await);
        });
    }

    #[wasm_bindgen_test]
    fn suggestion_db_updates_graph_values() {
        Fixture::set_up().run(|graph| async move {
            let mut sub = graph.subscribe();
            let update = language_server::types::SuggestionDatabaseUpdatesEvent {
                updates : vec![],
                current_version : default(),
            };
            graph.suggestion_db.apply_update_event(update);
            assert_eq!(Some(Notification::PortsUpdate), sub.next().await);
        });
    }

    #[wasm_bindgen_test]
    fn graph_controller_inline_definition() {
        let mut test = Fixture::set_up();
        const EXPRESSION:&str = "2+2";
        test.data.code = iformat!("main = {EXPRESSION}");
        test.run(|graph| async move {
            let nodes   = graph.nodes().unwrap();
            let (node,) = nodes.expect_tuple();
            assert_eq!(node.info.expression().repr(), EXPRESSION);
            let id   = node.info.id();
            let node = graph.node(id).unwrap();
            assert_eq!(node.info.expression().repr(), EXPRESSION);
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_block_definition() {
        let mut test  = Fixture::set_up();
        test.data.code = r"
main =
    foo = 2
    print foo".to_string();
        test.run(|graph| async move {
            let nodes         = graph.nodes().unwrap();
            let (node1,node2) = nodes.expect_tuple();
            assert_eq!(node1.info.expression().repr(), "2");
            assert_eq!(node2.info.expression().repr(), "print foo");
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_parse_expression() {
        let mut test  = Fixture::set_up();
        test.run(|graph| async move {
            let foo = graph.parse_node_expression("foo").unwrap();
            assert_eq!(expect_shape::<ast::Var>(&foo), &ast::Var {name:"foo".into()});

            assert!(graph.parse_node_expression("Vec").is_ok());
            assert!(graph.parse_node_expression("5").is_ok());
            assert!(graph.parse_node_expression("5+5").is_ok());
            assert!(graph.parse_node_expression("a+5").is_ok());
            assert!(graph.parse_node_expression("a=5").is_err());
        })
    }

    #[wasm_bindgen_test]
    fn span_tree_context_handling_metadata_and_name() {
        let entry     = crate::test::mock::data::suggestion_entry_foo();
        let mut test  = Fixture::set_up();
        test.data.suggestions.insert(0,entry.clone());
        test.data.code = "main = bar".to_owned();
        test.run(|graph| async move {
            let nodes = graph.nodes().unwrap();
            assert_eq!(nodes.len(),1);
            let id = nodes[0].info.id();
            graph.module.set_node_metadata(id,NodeMetadata {
                position        : None,
                intended_method : entry.method_id(),
            }).unwrap();

            let get_invocation_info = || {
                let node = &graph.nodes().unwrap()[0];
                assert_eq!(node.info.id(),id);
                let expression = node.info.expression().repr();
                graph.call_info(id, Some(expression.as_str()))
            };

            // Now node is `bar` while the intended method is `foo`.
            // No invocation should be reported, as the name is mismatched.
            assert!(get_invocation_info().is_none());

            // Now the name should be good and we should the information about node being a call.
            graph.set_expression(id,&entry.name).unwrap();
            crate::test::assert_call_info(get_invocation_info().unwrap(),&entry);

            // Now we remove metadata, so the information is no more.
            graph.module.remove_node_metadata(id).unwrap();
            assert!(get_invocation_info().is_none());
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_used_names_in_inline_def() {
        let mut test  = Fixture::set_up();
        test.data.code = "main = foo".into();
        test.run(|graph| async move {
            let expected_name = LocatedName::new_root(NormalizedName::new("foo"));
            let used_names    = graph.used_names().unwrap();
            assert_eq!(used_names, vec![expected_name]);
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_nested_definition() {
        let mut test  = Fixture::set_up();
        test.data.code = r"main =
    foo a =
        bar b = 5
    print foo".into();
        test.data.graph_id = definition::Id::new_plain_names(&["main","foo"]);
        test.run(|graph| async move {
            let expression = "new_node";
            graph.add_node(NewNodeInfo::new_pushed_back(expression)).unwrap();
            let expected_program = r"main =
    foo a =
        bar b = 5
        new_node
    print foo";
            model::module::test::expect_code(&*graph.module,expected_program);
        })
    }

    #[wasm_bindgen_test]
    fn collapsing_nodes_avoids_name_conflicts() {
        // Checks that generated name avoid collision with other methods defined in the module
        // and with symbols used that could be shadowed by the extracted method's name.
        let mut test  = Fixture::set_up();
        let code = r"
func2 = 454

main =
    a = 10
    b = 20
    c = a + b
    d = c + d
    a + func1";

        let expected_code = "
func2 = 454

func3 a =
    b = 20
    c = a + b
    d = c + d

main =
    a = 10
    here.func3 a
    a + func1";

        test.data.code = code.to_owned();
        test.run(move |graph| async move {
            let nodes = graph.nodes().unwrap();
            let selected_nodes = nodes[1..4].iter().map(|node| node.info.id());
            graph.collapse(selected_nodes,"func").unwrap();
            model::module::test::expect_code(&*graph.module,expected_code);
        })
    }

    #[wasm_bindgen_test]
    fn collapsing_nodes() {
        let mut test  = Fixture::set_up();
        let code = r"
main =
    a = 10
    b = 20
    a + c";

        let expected_code = "
func1 =
    a = 10
    b = 20
    a

main =
    a = here.func1
    a + c";

        test.data.code = code.to_owned();
        test.run(move |graph| async move {
            let nodes = graph.nodes().unwrap();
            assert_eq!(nodes.len(),3);
            graph.module.set_node_metadata(nodes[0].info.id(), NodeMetadata {
                position : Some(Position::new(100.0,200.0)),
                ..default()
            }).unwrap();
            graph.module.set_node_metadata(nodes[1].info.id(), NodeMetadata {
                position : Some(Position::new(150.0,300.0)),
                ..default()
            }).unwrap();

            let selected_nodes = nodes[0..2].iter().map(|node| node.info.id());
            let collapsed_node = graph.collapse(selected_nodes,"func").unwrap();
            model::module::test::expect_code(&*graph.module,expected_code);

            let nodes_after = graph.nodes().unwrap();
            assert_eq!(nodes_after.len(),2);
            let collapsed_node_info = graph.node(collapsed_node).unwrap();
            let collapsed_node_pos  = collapsed_node_info.metadata.and_then(|m| m.position);
            assert_eq!(collapsed_node_pos, Some(Position::new(125.0,250.0)));
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_doubly_nested_definition() {
        // Tests editing nested definition that requires transforming inline expression into
        // into a new block.
        let mut test  = Fixture::set_up();
        // Not using multi-line raw string literals, as we don't want IntelliJ to automatically
        // strip the trailing whitespace in the lines.
        test.data.code     = "main =\n    foo a =\n        bar b = 5\n    print foo".into();
        test.data.graph_id = definition::Id::new_plain_names(&["main","foo","bar"]);
        test.run(|graph| async move {
            let expression = "new_node";
            graph.add_node(NewNodeInfo::new_pushed_back(expression)).unwrap();
            let expected_program = "main =\n    foo a =\n        bar b = \
                                    \n            5\n            new_node\n    print foo";

            model::module::test::expect_code(&*graph.module,expected_program);
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_node_operations_node() {
        let mut test  = Fixture::set_up();
        const PROGRAM:&str = r"
main =
    foo = 2
    print foo";
        test.data.code = PROGRAM.into();
        test.run(|graph| async move {
            // === Initial nodes ===
            let nodes         = graph.nodes().unwrap();
            let (node1,node2) = nodes.expect_tuple();
            assert_eq!(node1.info.expression().repr(), "2");
            assert_eq!(node2.info.expression().repr(), "print foo");


            // === Add node ===
            let id       = ast::Id::new_v4();
            let position = Some(model::module::Position::new(10.0,20.0));
            let metadata = NodeMetadata {position,..default()};
            let info     = NewNodeInfo {
                expression        : "a+b".into(),
                metadata          : Some(metadata),
                id                : Some(id),
                location_hint     : LocationHint::End,
                introduce_pattern : false,
            };
            graph.add_node(info.clone()).unwrap();
            let expected_program = r"
main =
    foo = 2
    print foo
    a+b";

            model::module::test::expect_code(&*graph.module,expected_program);
            let nodes = graph.nodes().unwrap();
            let (_,_,node3) = nodes.expect_tuple();
            assert_eq!(node3.info.id(),id);
            assert_eq!(node3.info.expression().repr(), "a+b");
            let pos = node3.metadata.unwrap().position;
            assert_eq!(pos, position);
            assert!(graph.module.node_metadata(id).is_ok());


            // === Edit node ===
            graph.set_expression(id, "bar baz").unwrap();
            let (_,_,node3) = graph.nodes().unwrap().expect_tuple();
            assert_eq!(node3.info.id(),id);
            assert_eq!(node3.info.expression().repr(), "bar baz");
            assert_eq!(node3.metadata.unwrap().position, position);


            // === Remove node ===
            graph.remove_node(node3.info.id()).unwrap();
            let nodes = graph.nodes().unwrap();
            let (node1,node2) = nodes.expect_tuple();
            assert_eq!(node1.info.expression().repr(), "2");
            assert_eq!(node2.info.expression().repr(), "print foo");
            assert!(graph.module.node_metadata(id).is_err());

            model::module::test::expect_code(&*graph.module, PROGRAM);


            // === Test adding node with automatically generated pattern ===
            let info_w_pattern = NewNodeInfo {
                introduce_pattern : true,
                ..info
            };
            graph.add_node(info_w_pattern).unwrap();
            let expected_program = r"
main =
    foo = 2
    print foo
    sum1 = a+b";
            model::module::test::expect_code(&*graph.module,expected_program);
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_connections_listing() {
        let mut test  = Fixture::set_up();
        const PROGRAM:&str = r"
main =
    x,y = get_pos
    print x
    z = print $ foo y
    print z
    foo
        print z";
        test.data.code = PROGRAM.into();
        test.run(|graph| async move {
            let connections = connections(&graph).unwrap();

            let (node0,node1,node2,node3,node4) = graph.nodes().unwrap().expect_tuple();
            assert_eq!(node0.info.expression().repr(), "get_pos");
            assert_eq!(node1.info.expression().repr(), "print x");
            assert_eq!(node2.info.expression().repr(), "print $ foo y");
            assert_eq!(node3.info.expression().repr(), "print z");

            let c = &connections.connections[0];
            assert_eq!(c.source.node,      node0.info.id());
            assert_eq!(c.source.port,      span_tree::node::Crumbs::new(vec![1]));
            assert_eq!(c.destination.node, node1.info.id());
            assert_eq!(c.destination.port, span_tree::node::Crumbs::new(vec![2]));

            let c = &connections.connections[1];
            assert_eq!(c.source.node     , node0.info.id());
            assert_eq!(c.source.port     , span_tree::node::Crumbs::new(vec![4]));
            assert_eq!(c.destination.node, node2.info.id());
            assert_eq!(c.destination.port, span_tree::node::Crumbs::new(vec![4,2]));

            let c = &connections.connections[2];
            assert_eq!(c.source.node     , node2.info.id());
            assert_eq!(c.source.port     , span_tree::node::Crumbs::default());
            assert_eq!(c.destination.node, node3.info.id());
            assert_eq!(c.destination.port, span_tree::node::Crumbs::new(vec![2]));

            use ast::crumbs::*;
            let c = &connections.connections[3];
            assert_eq!(c.source.node     , node2.info.id());
            assert_eq!(c.source.port     , span_tree::node::Crumbs::default());
            assert_eq!(c.destination.node, node4.info.id());
            assert_eq!(c.destination.port, span_tree::node::Crumbs::new(vec![2]));
            assert_eq!(c.destination.var_crumbs, crumbs!(BlockCrumb::HeadLine,PrefixCrumb::Arg));
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_create_connection() {
        /// A case for creating connection test. The field's names are short to be able to write
        /// nice-to-read table of cases without very long lines (see `let cases` below).
        #[derive(Clone,Debug)]
        struct Case {
            /// A pattern (the left side of assignment operator) of source node.
            src      : &'static str,
            /// An expression of destination node.
            dst      : &'static str,
            /// Crumbs of source and destination ports (i.e. SpanTree nodes)
            ports    : (&'static [usize],&'static [usize]),
            /// Expected destination expression after connecting.
            expected : &'static str,
        }

        impl Case {
            fn run(&self) {
                let mut test    = Fixture::set_up();
                let main_prefix = format!("main = \n    {} = foo\n    ",self.src);
                let main        = format!("{}{}",main_prefix,self.dst);
                let expected    = format!("{}{}",main_prefix,self.expected);
                let this        = self.clone();

                let (src_port,dst_port) = self.ports;
                let src_port = src_port.to_vec();
                let dst_port = dst_port.to_vec();

                test.data.code = main;
                test.run(|graph| async move {
                    let (node0,node1) = graph.nodes().unwrap().expect_tuple();
                    let source        = Endpoint::new(node0.info.id(),src_port.to_vec());
                    let destination   = Endpoint::new(node1.info.id(),dst_port.to_vec());
                    let connection    = Connection{source,destination};
                    graph.connect(&connection,&span_tree::generate::context::Empty).unwrap();
                    let new_main = graph.graph_definition_info().unwrap().ast.repr();
                    assert_eq!(new_main,expected,"Case {:?}",this);
                })
            }
        }

        let cases = &
            [ Case {src:"x"      , dst:"foo"      , expected:"x"         , ports:(&[]   ,&[]   )}
            , Case {src:"x,y"    , dst:"foo a"    , expected:"foo y"     , ports:(&[4]  ,&[2]  )}
            , Case {src:"Vec x y", dst:"1 + 2 + 3", expected:"x + 2 + 3" , ports:(&[0,2],&[0,1])}
            ];
        for case in cases {
            case.run()
        }
    }

    #[wasm_bindgen_test]
    fn graph_controller_create_connection_reordering() {
        let mut test  = Fixture::set_up();
        const PROGRAM:&str = r"main =
    sum = _ + _
    a = 1
    b = 3";
        const EXPECTED:&str = r"main =
    a = 1
    b = 3
    sum = _ + b";
        test.data.code = PROGRAM.into();
        test.run(|graph| async move {
            assert!(connections(&graph).unwrap().connections.is_empty());
            let (node0,_node1,node2) = graph.nodes().unwrap().expect_tuple();
            let connection_to_add = Connection {
                source : Endpoint {
                    node      : node2.info.id(),
                    port      : default(),
                    var_crumbs: default()
                },
                destination : Endpoint {
                    node      : node0.info.id(),
                    port      : vec![4].into(),
                    var_crumbs: default()
                }
            };
            graph.connect(&connection_to_add,&span_tree::generate::context::Empty).unwrap();
            let new_main = graph.graph_definition_info().unwrap().ast.repr();
            assert_eq!(new_main,EXPECTED);
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_create_connection_reordering_with_dependency() {
        let mut test  = Fixture::set_up();
        const PROGRAM:&str = r"main =
    sum = _ + _
    IO.println sum
    a = 1
    b = sum + 2
    c = 3
    d = 4";
        const EXPECTED:&str = r"main =
    a = 1
    c = 3
    sum = _ + c
    IO.println sum
    b = sum + 2
    d = 4";
        test.data.code = PROGRAM.into();
        test.run(|graph| async move {
            let (node0,_node1,_node2,_node3,node4,_node5) = graph.nodes().unwrap().expect_tuple();
            let connection_to_add = Connection {
                source : Endpoint {
                    node      : node4.info.id(),
                    port      : default(),
                    var_crumbs: default()
                },
                destination : Endpoint {
                    node      : node0.info.id(),
                    port      : vec![4].into(),
                    var_crumbs: default()
                }
            };
            graph.connect(&connection_to_add,&span_tree::generate::context::Empty).unwrap();
            let new_main = graph.graph_definition_info().unwrap().ast.repr();
            assert_eq!(new_main,EXPECTED);
        })
    }

    #[wasm_bindgen_test]
    fn graph_controller_create_connection_introducing_var() {
        let mut test  = Fixture::set_up();
        const PROGRAM:&str = r"main =
    calculate
    print _
    calculate1 = calculate2
    calculate3 calculate5 = calculate5 calculate4";
        test.data.code = PROGRAM.into();
        // Note: we expect that name `calculate5` will be introduced. There is no conflict with a
        // function argument, as it just shadows outer variable.
        const EXPECTED:&str = r"main =
    calculate5 = calculate
    print calculate5
    calculate1 = calculate2
    calculate3 calculate5 = calculate5 calculate4";
        test.run(|graph| async move {
            assert!(connections(&graph).unwrap().connections.is_empty());
            let (node0,node1,_) = graph.nodes().unwrap().expect_tuple();
            let connection_to_add = Connection {
                source : Endpoint {
                    node      : node0.info.id(),
                    port      : default(),
                    var_crumbs: default()
                },
                destination : Endpoint {
                    node      : node1.info.id(),
                    port      : vec![2].into(), // `_` in `print _`
                    var_crumbs: default()
                }
            };
            graph.connect(&connection_to_add,&span_tree::generate::context::Empty).unwrap();
            let new_main = graph.graph_definition_info().unwrap().ast.repr();
            assert_eq!(new_main,EXPECTED);
        })
    }

    #[wasm_bindgen_test]
    fn suggested_names() {
        let parser = Parser::new_or_panic();
        let cases = [
            ("a+b",           "sum"),
            ("a-b",           "difference"),
            ("a*b",           "product"),
            ("a/b",           "quotient"),
            ("read 'foo.csv'","read"),
            ("Read 'foo.csv'","read"),
            ("574",           "number"),
            ("'Hello'",       "text"),
            ("'Hello",        "text"),
            ("\"Hello\"",     "text"),
            ("\"Hello",       "text"),
        ];

        for (code,expected_name) in &cases {
            let ast = parser.parse_line(*code).unwrap();
            let node = NodeInfo::from_line_ast(&ast).unwrap();
            let name = Handle::variable_name_base_for(&node);
            assert_eq!(&name,expected_name);
        }
    }

    #[wasm_bindgen_test]
    fn disconnect() {
        #[derive(Clone,Debug)]
        struct Case {
            dest_node_expr     : &'static str,
            dest_node_expected : &'static str,
        }

        impl Case {
            fn run(&self) {
                let mut test  = Fixture::set_up();
                const MAIN_PREFIX:&str = "main = \n    var = foo\n    ";
                test.data.code = format!("{}{}",MAIN_PREFIX,self.dest_node_expr);
                let expected   = format!("{}{}",MAIN_PREFIX,self.dest_node_expected);
                let this       = self.clone();
                test.run(|graph| async move {
                    let connections = connections(&graph).unwrap();
                    let connection  = connections.connections.first().unwrap();
                    graph.disconnect(connection,&span_tree::generate::context::Empty).unwrap();
                    let new_main = graph.graph_definition_info().unwrap().ast.repr();
                    assert_eq!(new_main,expected,"Case {:?}",this);
                })
            }
        }

        let cases = &
            [ Case {dest_node_expr:"foo var"             , dest_node_expected:"foo _"              }
            , Case {dest_node_expr:"foo var a"           , dest_node_expected:"foo _ a"            }
            , Case {dest_node_expr:"foo a var"           , dest_node_expected:"foo a"              }
            , Case {dest_node_expr:"var + a"             , dest_node_expected:"_ + a"              }
            , Case {dest_node_expr:"a + var"             , dest_node_expected:"a + _"              }
            , Case {dest_node_expr:"var + b + c"         , dest_node_expected:"_ + b + c"          }
            , Case {dest_node_expr:"a + var + c"         , dest_node_expected:"a + _ + c"          }
            , Case {dest_node_expr:"a + b + var"         , dest_node_expected:"a + b"              }
            , Case {dest_node_expr:"var , a"             , dest_node_expected:"_ , a"              }
            , Case {dest_node_expr:"a , var"             , dest_node_expected:"a , _"              }
            , Case {dest_node_expr:"var , b , c"         , dest_node_expected:"_ , b , c"          }
            , Case {dest_node_expr:"a , var , c"         , dest_node_expected:"a , _ , c"          }
            , Case {dest_node_expr:"a , b , var"         , dest_node_expected:"a , b"              }
            , Case {dest_node_expr:"f\n        bar a var", dest_node_expected: "f\n        bar a _"}
            ];
        for case in cases {
            case.run();
        }
    }
}

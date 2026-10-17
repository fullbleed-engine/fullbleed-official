# SPDX-License-Identifier: MIT
"""Bootstrap 5.0.0 vocabulary shared by the kit, docs and coverage checks."""

VERSION = "5.0.0"
CSS_SHA256 = "04083fcecdd9e19b2acd1e0073437aa07c98d230f0198d7f1fd470ab220fef64"
BREAKPOINTS = ("sm", "md", "lg", "xl", "xxl")
COLORS = ("primary", "secondary", "success", "danger", "warning", "info", "light", "dark")
SIZES = ("sm", "lg")

# The 23 component families in the v5.0.0 tag's site/data/sidebar.yml.
COMPONENT_FAMILIES = {
    "accordion": ("Accordion", "AccordionItem"),
    "alerts": ("Alert",),
    "badge": ("Badge",),
    "breadcrumb": ("Breadcrumb", "BreadcrumbItem"),
    "buttons": ("Button",),
    "button-group": ("ButtonGroup", "ButtonToolbar"),
    "card": ("Card", "CardBody", "CardHeader", "CardFooter", "CardTitle", "CardSubtitle", "CardText", "CardImage", "CardGroup"),
    "carousel": ("Carousel", "CarouselItem", "CarouselCaption"),
    "close-button": ("CloseButton",),
    "collapse": ("Collapse",),
    "dropdowns": ("Dropdown", "DropdownItem", "DropdownDivider", "DropdownHeader"),
    "list-group": ("ListGroup", "ListGroupItem"),
    "modal": ("Modal", "ModalHeader", "ModalBody", "ModalFooter", "ModalTitle"),
    "navs-tabs": ("Nav", "NavItem", "NavLink", "Tabs", "Tab", "TabContent", "TabPane"),
    "navbar": ("Navbar", "NavbarBrand", "NavbarText", "NavbarNav"),
    "offcanvas": ("Offcanvas",),
    "pagination": ("Pagination", "PageItem"),
    "popovers": ("Popover",),
    "progress": ("Progress", "ProgressBar"),
    "scrollspy": ("Scrollspy",),
    "spinners": ("Spinner",),
    "toasts": ("Toast",),
    "tooltips": ("Tooltip",),
}

PRIMITIVES = {
    "Box": ("div", ""), "Text": ("span", ""), "Paragraph": ("p", ""),
    "Strong": ("strong", ""), "Emphasis": ("em", ""),
    "Small": ("small", ""), "Mark": ("mark", ""), "Code": ("code", ""),
    "Pre": ("pre", ""), "Kbd": ("kbd", ""), "Blockquote": ("blockquote", "blockquote"),
    "BlockquoteFooter": ("figcaption", "blockquote-footer"),
    "Figure": ("figure", "figure"), "FigureCaption": ("figcaption", "figure-caption"),
    "CardBody": ("div", "card-body"), "CardHeader": ("div", "card-header"),
    "CardFooter": ("div", "card-footer"), "CardTitle": ("h2", "card-title"),
    "CardSubtitle": ("h3", "card-subtitle"), "CardText": ("p", "card-text"),
    "CardGroup": ("div", "card-group"), "ButtonToolbar": ("div", "btn-toolbar"),
    "CarouselCaption": ("div", "carousel-caption"),
    "DropdownHeader": ("li", "dropdown-header"),
    "ModalHeader": ("div", "modal-header"), "ModalBody": ("div", "modal-body"),
    "ModalFooter": ("div", "modal-footer"), "ModalTitle": ("h2", "modal-title"),
    "NavItem": ("li", "nav-item"), "TabContent": ("div", "tab-content"),
    "NavbarText": ("span", "navbar-text"), "NavbarNav": ("ul", "navbar-nav"),
    "Form": ("form", ""), "Label": ("label", "form-label"), "FormText": ("div", "form-text"),
    "Fieldset": ("fieldset", ""), "Legend": ("legend", ""),
    "InputGroupText": ("span", "input-group-text"), "Option": ("option", ""),
    "Datalist": ("datalist", ""), "Optgroup": ("optgroup", ""),
    "TableHead": ("thead", ""), "TableBody": ("tbody", ""), "TableFoot": ("tfoot", ""),
    "TableRow": ("tr", ""), "TableHeader": ("th", ""), "TableCell": ("td", ""),
    "TableCaption": ("caption", ""),
    "CardImageOverlay": ("div", "card-img-overlay"),
    "Address": ("address", ""), "DescriptionList": ("dl", ""),
    "Term": ("dt", ""), "Definition": ("dd", ""),
    "List": ("ul", ""), "OrderedList": ("ol", ""), "ListItem": ("li", ""),
    "Rule": ("hr", ""), "Break": ("br", ""),
}

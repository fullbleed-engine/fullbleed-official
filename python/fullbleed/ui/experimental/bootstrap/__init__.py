# SPDX-License-Identifier: MIT
"""Bootstrap 5.0.0 for static documents and HTML, with owned, bundled CSS.

Components return immutable experimental Nodes. Group items (Tab,
AccordionItem and CarouselItem) are immutable descriptors consumed by their
parent. No component injects JavaScript, events, CDNs or network dependencies.
"""
from .._table import Column as Column
from .._tree import component as _component
from ._core import classes as classes, primitive as _primitive, stylesheet as stylesheet
from ._schema import VERSION as VERSION, COMPONENT_FAMILIES as COMPONENT_FAMILIES, PRIMITIVES as _PRIMITIVES
from ._utilities import class_names as class_names, utilities as utilities
from ._layout import (
    Container, Row, Col, Heading, Link, Image, FigureImage, Table, DataTable, Ratio, VisuallyHidden,
)
from ._forms import Input, Textarea, Select, Check, Range, InputGroup, FloatingLabel, Feedback
from ._components import (
    Alert, Badge, Button, ButtonGroup, CloseButton, Breadcrumb, BreadcrumbItem,
    Card, CardImage, Collapse, AccordionItem, Accordion, Dropdown, DropdownItem,
    DropdownDivider, ListGroup, ListGroupItem, Modal, Nav, NavLink, Tab, Tabs, TabPane,
    Navbar, NavbarBrand, Offcanvas, Pagination, PageItem, Popover, Tooltip, Progress,
    ProgressBar, Scrollspy, Spinner, Toast, CarouselItem, Carousel,
)

for _name, (_tag, _class_name) in _PRIMITIVES.items():
    globals()[_name] = _component(_primitive(_name, _tag, _class_name))

__all__ = [
    "VERSION", "COMPONENT_FAMILIES", "Column", "classes", "stylesheet", "class_names", "utilities",
    "Container", "Row", "Col", "Heading", "Link", "Image", "FigureImage", "Table", "DataTable", "Ratio", "VisuallyHidden",
    "Input", "Textarea", "Select", "Check", "Range", "InputGroup", "FloatingLabel", "Feedback",
    "Alert", "Badge", "Button", "ButtonGroup", "CloseButton", "Breadcrumb", "BreadcrumbItem",
    "Card", "CardImage", "Collapse", "AccordionItem", "Accordion", "Dropdown", "DropdownItem",
    "DropdownDivider", "ListGroup", "ListGroupItem", "Modal", "Nav", "NavLink", "Tab", "Tabs", "TabPane",
    "Navbar", "NavbarBrand", "Offcanvas", "Pagination", "PageItem", "Popover", "Tooltip", "Progress",
    "ProgressBar", "Scrollspy", "Spinner", "Toast", "CarouselItem", "Carousel", *_PRIMITIVES,
]

del _name, _tag, _class_name
